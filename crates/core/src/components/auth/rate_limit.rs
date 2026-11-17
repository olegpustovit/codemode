use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::time::Instant;
use uuid::Uuid;

const CAPACITY: usize = 4096;
const WINDOW: Duration = Duration::from_secs(60);
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Session(Uuid),
    Token(Uuid),
    Endpoint(Uuid),
}
struct Bucket {
    used: u32,
    expires: Instant,
}
pub struct RateLimiter {
    per_session: u32,
    per_token: u32,
    per_endpoint: u32,
    buckets: Mutex<HashMap<Key, Bucket>>,
}
impl RateLimiter {
    pub fn new(per_session: u32, per_token: u32, per_endpoint: u32) -> Self {
        Self {
            per_session,
            per_token,
            per_endpoint,
            buckets: Mutex::new(HashMap::new()),
        }
    }
    /// All applicable limits admit together; rejection consumes no budget.
    pub fn check(&self, token: Uuid, endpoint: Uuid) -> Result<(), u64> {
        self.admit(&[
            (Key::Token(token), self.per_token),
            (Key::Endpoint(endpoint), self.per_endpoint),
        ])
    }
    pub fn check_session(&self, session: Uuid, token: Uuid, endpoint: Uuid) -> Result<(), u64> {
        self.admit(&[
            (Key::Session(session), self.per_session),
            (Key::Token(token), self.per_token),
            (Key::Endpoint(endpoint), self.per_endpoint),
        ])
    }
    fn admit(&self, keys: &[(Key, u32)]) -> Result<(), u64> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().map_err(|_| 60u64)?;
        let needed = keys
            .iter()
            .filter(|(key, _)| !buckets.contains_key(key))
            .count();
        if buckets.len() + needed > CAPACITY {
            buckets.retain(|_, bucket| bucket.expires > now);
            let needed = keys
                .iter()
                .filter(|(key, _)| !buckets.contains_key(key))
                .count();
            if buckets.len() + needed > CAPACITY {
                return Err(60);
            }
        }
        let mut retry = 0;
        for (key, limit) in keys {
            if let Some(bucket) = buckets.get_mut(key) {
                if bucket.expires <= now {
                    bucket.used = 0;
                    bucket.expires = now + WINDOW;
                }
                if bucket.used >= *limit {
                    let left = bucket.expires - now;
                    retry = retry.max(left.as_secs() + u64::from(left.subsec_nanos() > 0));
                }
            } else if *limit == 0 {
                return Err(60);
            }
        }
        if retry > 0 {
            return Err(retry);
        }
        for &(key, _) in keys {
            let bucket = buckets.entry(key).or_insert(Bucket {
                used: 0,
                expires: now + WINDOW,
            });
            bucket.used += 1;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn token_and_endpoint_admission_is_atomic_and_recovers_after_expiry() {
        let limiter = RateLimiter::new(2, 2, 3);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let endpoint = Uuid::new_v4();
        assert!(limiter.check(a, endpoint).is_ok());
        assert!(limiter.check(a, endpoint).is_ok());
        assert_eq!(limiter.check(a, endpoint), Err(60));
        assert!(limiter.check(b, endpoint).is_ok());
        assert_eq!(limiter.check(b, endpoint), Err(60));
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(limiter.check(a, endpoint), Err(1));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(limiter.check(a, endpoint).is_ok());
        assert!(limiter.check(b, endpoint).is_ok());
    }
    #[tokio::test(start_paused = true)]
    async fn sessions_have_independent_buckets_under_a_shared_token_ceiling() {
        let limiter = RateLimiter::new(2, 3, 10);
        let token = Uuid::new_v4();
        let endpoint = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(limiter.check_session(a, token, endpoint).is_ok());
        assert!(limiter.check_session(a, token, endpoint).is_ok());
        assert_eq!(limiter.check_session(a, token, endpoint), Err(60));
        assert!(limiter.check_session(b, token, endpoint).is_ok());
        assert_eq!(limiter.check_session(b, token, endpoint), Err(60));
        tokio::time::advance(WINDOW).await;
        assert!(limiter.check_session(a, token, endpoint).is_ok());
        assert!(limiter.check_session(b, token, endpoint).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn state_capacity_is_bounded_without_evicting_active_callers() {
        let limiter = RateLimiter::new(2, 2, 2);
        let first = (Uuid::new_v4(), Uuid::new_v4());
        assert!(limiter.check(first.0, first.1).is_ok());
        for _ in 1..CAPACITY / 2 {
            assert!(limiter.check(Uuid::new_v4(), Uuid::new_v4()).is_ok());
        }
        assert_eq!(limiter.buckets.lock().unwrap().len(), CAPACITY);
        assert_eq!(limiter.check(Uuid::new_v4(), Uuid::new_v4()), Err(60));
        assert!(limiter.check(first.0, first.1).is_ok());
        assert_eq!(limiter.check(first.0, first.1), Err(60));
        tokio::time::advance(WINDOW).await;
        assert!(limiter.check(Uuid::new_v4(), Uuid::new_v4()).is_ok());
        assert!(limiter.buckets.lock().unwrap().len() <= CAPACITY);
    }
}
