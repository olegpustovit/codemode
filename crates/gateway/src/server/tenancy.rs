use super::*;
use codemode_core::components::{accounts::Accounts, sources::Sources, workspaces::Workspaces};
use std::collections::HashMap;
use tokio::sync::{Mutex, OnceCell};
use tower::ServiceExt;
use uuid::Uuid;

struct Tenants {
    template: State,
    routers: Mutex<HashMap<Uuid, Arc<OnceCell<Router>>>>,
}

pub(super) fn router(state: State) -> Router {
    let tenants = Arc::new(Tenants {
        template: state.clone(),
        routers: Mutex::new(HashMap::new()),
    });
    // Select the tenant before matching paths, so ID parameters are extracted only once.
    Router::new()
        .fallback_service(routes(state))
        .layer(middleware::from_fn_with_state(tenants, select))
}

impl Tenants {
    async fn get(&self, tenant: Uuid) -> anyhow::Result<Router> {
        let cell = self.routers.lock().await.entry(tenant).or_default().clone();
        Ok(cell.get_or_try_init(|| self.build(tenant)).await?.clone())
    }
    async fn build(&self, tenant: Uuid) -> anyhow::Result<Router> {
        let base = &self.template;
        let db = base.db.for_tenant(tenant).await?;
        let accounts: Accounts = base.accounts.with_database(db.clone());
        let endpoints: Workspaces = base.workspaces.with_database(db.clone());
        endpoints.initialize().await?;
        let catalogue = Catalogue::new(db.clone());
        let sources = Sources::new(
            db.clone(),
            catalogue.clone(),
            accounts.clone(),
            endpoints.clone(),
        )
        .with_access(base.access.clone(), tenant);
        let stop = base.stop.child_token();
        let guard = stop.clone().drop_guard();
        let execution = Arc::new(
            base.execution
                .with_database(db.clone(), sources.clone(), stop),
        );
        execution.start_sources();
        tokio::time::timeout(std::time::Duration::from_secs(35), sources.ready()).await?;
        let state = State {
            db: db.clone(),
            auth: Auth::new(db.clone()).with_access(base.access.clone()),
            catalogue,
            execution,
            accounts: Arc::new(accounts),
            sources: Arc::new(sources),
            workspaces: Arc::new(endpoints),
            oauth: Arc::new(base.oauth.with_database(db)),
            ..base.clone()
        };
        let router = routes(state);
        guard.disarm();
        Ok(router)
    }
}

async fn select(
    AxumState(tenants): AxumState<Arc<Tenants>>,
    request: Request,
    next: Next,
) -> Response {
    let base = &tenants.template;
    let path = request.uri().path();
    let tenant = if path.starts_with("/api/")
        && !matches!(path, "/api/config" | "/api/session" | "/api/register")
    {
        match crate::admin::cookie(request.headers(), base.identity_enabled) {
            Some(token) => crate::admin::browser_user(&base.sessions, base.identity_enabled, token)
                .await
                .map(|u| u.map(|u| u.tenant_id)),
            None => Ok(None),
        }
    } else if matches!(path, "/mcp" | "/mcp/" | "/mcp-admin" | "/mcp-admin/") {
        let headers = request.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .and_then(codemode_core::components::workspaces::authority);
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, t)| t.trim_start_matches(' '));
        match (host, bearer) {
            (Some(host), Some(token))
                if headers.get_all(header::HOST).iter().count() == 1
                    && headers.get_all(header::AUTHORIZATION).iter().count() == 1 =>
            {
                base.auth
                    .authenticate(token, &host)
                    .await
                    .map(|p| p.map(|p| p.tenant_id))
                    .map_err(anyhow::Error::from)
            }
            _ => Ok(None),
        }
    } else {
        Ok(None)
    };
    match tenant {
        Ok(Some(tenant)) => {
            crate::access::request(&base.access, tenant, async {
                match tenants.get(tenant).await {
                    Ok(router) => router
                        .oneshot(request)
                        .await
                        .unwrap_or_else(|never| match never {}),
                    Err(error) => {
                        tracing::error!(error=?error,"tenant initialization failed");
                        (StatusCode::SERVICE_UNAVAILABLE, "tenant unavailable").into_response()
                    }
                }
            })
            .await
        }
        Ok(None) => next.run(request).await,
        Err(error) => {
            if let Some(codemode_core::components::auth::Error::Access(error)) =
                error.downcast_ref::<codemode_core::components::auth::Error>()
            {
                return crate::access::response(*error);
            }
            tracing::error!(error=?error,"tenant authentication failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication unavailable",
            )
                .into_response()
        }
    }
}
