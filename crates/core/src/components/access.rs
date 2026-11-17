//! Central eligibility supplements tenant isolation, workspace scopes and policy.
//! The private owner/subject binding is read through the composition root's pool;
//! tenant database roles never gain access to identity tables.
use crate::infrastructure::postgres::Database;
use idclient::AccessClient;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

const CHECK_BOUND: Duration = Duration::from_secs(2);
const RECHECK: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("Microapps ID access denied")]
    Denied,
    #[error("Microapps ID access unavailable")]
    Unavailable,
}

/// Disabled by default, with no identity network or binding lookup when disabled.
#[derive(Clone, Default)]
pub struct Access(Option<Arc<Enabled>>);

struct Enabled {
    db: Database,
    client: AccessClient,
    stop: CancellationToken,
    tasks: TaskTracker,
}

impl Access {
    pub fn new(
        db: Database,
        client: AccessClient,
        stop: CancellationToken,
        tasks: TaskTracker,
    ) -> Self {
        Self(Some(Arc::new(Enabled {
            db,
            client,
            stop,
            tasks,
        })))
    }

    pub fn enforced(&self) -> bool {
        self.0.is_some()
    }

    /// Callback claims have already been verified by the OIDC SDK. Check before
    /// any local account linking/provisioning or session replacement.
    pub async fn subject_fresh(&self, subject: &str) -> Result<(), Error> {
        let Some(inner) = &self.0 else {
            return Ok(());
        };
        bounded(async {
            let decision = inner
                .client
                .check_fresh(subject)
                .await
                .map_err(|_| Error::Unavailable)?;
            if decision.allowed {
                Ok(())
            } else {
                Err(Error::Denied)
            }
        })
        .await
    }

    async fn tenant_subject(
        &self,
        tenant: Uuid,
        pinned: Option<&str>,
        fresh: bool,
    ) -> Result<Option<String>, Error> {
        let Some(inner) = &self.0 else {
            return Ok(None);
        };
        bounded(async {
            // users.tenant_id and user_identities.user_id are both UNIQUE.
            // Missing owners/bindings, including the bootstrap nil tenant, deny.
            let subject = sqlx::query_scalar!("SELECT i.subject FROM users u JOIN user_identities i ON i.user_id=u.id WHERE u.tenant_id=$1", tenant)
                .fetch_optional(&inner.db.0).await.map_err(|_| Error::Unavailable)?.ok_or(Error::Denied)?;
            if pinned.is_some_and(|p| p != subject) { return Err(Error::Denied); }
            let decision = if fresh { inner.client.check_fresh(&subject).await } else { inner.client.check(&subject).await }
                .map_err(|_| Error::Unavailable)?;
            if !decision.allowed { return Err(Error::Denied); }
            Ok(Some(subject))
        }).await
    }

    pub async fn tenant(&self, tenant: Uuid, fresh: bool) -> Result<(), Error> {
        self.tenant_subject(tenant, None, fresh).await.map(|_| ())
    }

    pub async fn workspace(&self, workspace: Uuid) -> Result<(), Error> {
        let Some(inner) = &self.0 else {
            return Ok(());
        };
        bounded(async {
            let tenant =
                sqlx::query_scalar!("SELECT tenant_id FROM endpoints WHERE id=$1", workspace)
                    .fetch_optional(&inner.db.0)
                    .await
                    .map_err(|_| Error::Unavailable)?
                    .ok_or(Error::Denied)?;
            self.tenant(tenant, true).await
        })
        .await
    }

    pub async fn grant(&self, grant: Uuid) -> Result<(), Error> {
        let Some(inner) = &self.0 else {
            return Ok(());
        };
        bounded(async {
            let tenant = sqlx::query_scalar!("SELECT e.tenant_id FROM oauth_grants g JOIN endpoints e ON e.id=g.endpoint_id WHERE g.id=$1", grant)
                .fetch_optional(&inner.db.0).await.map_err(|_| Error::Unavailable)?.ok_or(Error::Denied)?;
            self.tenant(tenant, true).await
        }).await
    }

    pub(crate) async fn source_watch(&self, source: Uuid) -> Result<Watch, Error> {
        let Some(inner) = &self.0 else {
            return self.watch(Uuid::nil()).await;
        };
        bounded(async {
            let tenant = sqlx::query_scalar!("SELECT tenant_id FROM sources WHERE id=$1", source)
                .fetch_optional(&inner.db.0)
                .await
                .map_err(|_| Error::Unavailable)?
                .ok_or(Error::Denied)?;
            self.watch(tenant).await
        })
        .await
    }

    /// One lease belongs to the original subject for the work's entire lifetime.
    /// Checks run independently while work is idle or waiting on an upstream.
    pub async fn watch(&self, tenant: Uuid) -> Result<Watch, Error> {
        let subject = self.tenant_subject(tenant, None, true).await?;
        let cancel = CancellationToken::new();
        let (failure, receiver) = watch::channel(None);
        let Some(inner) = &self.0 else {
            return Ok(Watch {
                cancel,
                task: None,
                receiver,
                _disabled: Some(failure),
            });
        };
        let access = self.clone();
        let stop = inner.stop.clone();
        let finish = cancel.clone();
        let task = inner.tasks.spawn(async move {
            loop {
                tokio::select! { biased;
                    _=finish.cancelled()=>return,
                    _=stop.cancelled()=>{failure.send_replace(Some(Error::Unavailable)); return;},
                    _=tokio::time::sleep(RECHECK)=>{},
                }
                let outcome = tokio::select! { biased;
                    _=finish.cancelled()=>return,
                    _=stop.cancelled()=>Err(Error::Unavailable),
                    result=access.tenant_subject(tenant, subject.as_deref(), true)=>result.map(|_| ()),
                };
                if let Err(error) = outcome { failure.send_replace(Some(error)); return; }
            }
        });
        Ok(Watch {
            cancel,
            task: Some(task),
            receiver,
            _disabled: None,
        })
    }

    pub async fn run<T>(&self, tenant: Uuid, future: impl Future<Output = T>) -> Result<T, Error> {
        let mut watch = self.watch(tenant).await?;
        let result = tokio::select! { biased;
            error=watch.stopped()=>Err(error),
            value=future=>Ok(value),
        };
        watch.close().await;
        result
    }
}

async fn bounded<T>(future: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    tokio::time::timeout(CHECK_BOUND, future)
        .await
        .map_err(|_| Error::Unavailable)?
}

pub struct Watch {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    receiver: watch::Receiver<Option<Error>>,
    _disabled: Option<watch::Sender<Option<Error>>>,
}
impl Watch {
    pub async fn stopped(&mut self) -> Error {
        loop {
            if let Some(error) = *self.receiver.borrow_and_update() {
                return error;
            }
            // A checker panic/early exit must never keep granting work.
            if self.receiver.changed().await.is_err() {
                return Error::Unavailable;
            }
        }
    }
    pub async fn close(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Only read-only DB/ID checks run in this task. Aborting also drops the
        // outbound request when the caller is itself cancelled.
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
