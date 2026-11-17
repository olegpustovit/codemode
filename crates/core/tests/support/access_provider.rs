//! Owned loopback policy transport. These tests exercise the published SDK;
//! native-provider qualification separately covers ID's real evaluator.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use codemode_core::{components::access::Access, infrastructure::postgres::Database};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct Policy {
    pub mode: Arc<AtomicU8>,
    pub checks: Arc<AtomicUsize>,
}
pub struct Provider {
    pub policy: Policy,
    pub access: Access,
    pub stop: CancellationToken,
    pub tasks: TaskTracker,
    server: tokio::task::JoinHandle<()>,
}
impl Provider {
    pub async fn new(db: Database) -> Self {
        let policy = Policy::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/api/access/check", post(check))
            .with_state(policy.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let stop = CancellationToken::new();
        let tasks = TaskTracker::new();
        let client = idclient::AccessClient::new(
            &issuer,
            "eval-client".into(),
            "eval-secret".into(),
            Duration::from_secs(25),
        )
        .unwrap();
        let access = Access::new(db, client, stop.clone(), tasks.clone());
        Self {
            policy,
            access,
            stop,
            tasks,
            server,
        }
    }
    pub fn mode(&self, mode: u8) {
        self.policy.mode.store(mode, Ordering::SeqCst);
    }
    pub async fn close(&self) {
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.cancel();
        self.server.abort();
    }
}
async fn check(
    State(policy): State<Policy>,
    headers: HeaderMap,
    Json(input): Json<serde_json::Value>,
) -> Response {
    use base64::{Engine, engine::general_purpose::STANDARD};
    assert_eq!(
        headers["authorization"],
        format!("Basic {}", STANDARD.encode("eval-client:eval-secret"))
    );
    policy.checks.fetch_add(1, Ordering::SeqCst);
    let subject = input["user_id"].as_str().unwrap();
    let mode = if subject == "usr_eval" {
        policy.mode.load(Ordering::SeqCst)
    } else {
        0
    };
    if mode == 2 {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if mode == 3 {
        return std::future::pending().await;
    }
    Json(serde_json::json!({"subject":subject,"client_id":"eval-client","allowed":mode==0,"reason_code":if mode==0 {"group_grant"} else {"user_block"},"policy_revision":1,"lease_seconds":60})).into_response()
}

pub async fn owner(pool: &sqlx::PgPool, tenant: Uuid, subject: &str) -> Uuid {
    sqlx::query("INSERT INTO tenants(id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    let user = sqlx::query_scalar("INSERT INTO users(email,password_hash,tenant_id,admin) VALUES($1,'identity-only',$2,$3) RETURNING id")
        .bind(format!("eval-{subject}@example.test")).bind(tenant).bind(tenant.is_nil()).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES($1,$2)")
        .bind(subject)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    user
}
