use super::admin::{State, router};
use axum::http::StatusCode;
use codemode_core::{
    components::{
        accounts::Accounts, auth::Auth, execution::log, sessions::Sessions, sources::Sources,
    },
    infrastructure::postgres::Database,
};
use sqlx::ConnectOptions;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
const CLEAR_COOKIE: &str =
    "__Host-codemode_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0";

#[sqlx::test(migrations = "../core/migrations")]
async fn browser_api_requires_its_own_session_and_rejects_cross_origin_writes(pool: sqlx::PgPool) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let auth = Auth::new(db.clone());
    let (agent_id, agent_token) = auth.create("agent", "test.example").await.unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let endpoints =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    endpoints.initialize().await.unwrap();
    let sources = Sources::new(
        db.clone(),
        codemode_core::components::catalogue::Catalogue::new(db.clone()),
        accounts.clone(),
        endpoints.clone(),
    );
    let sessions = Sessions::new(db.clone());
    sessions
        .register("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap();
    let state = State {
        agent_sessions: codemode_core::components::agent_sessions::AgentSessions::new(db.clone()),
        scripts: codemode_core::components::scripts::Scripts::new(db.clone()),
        oauth: Arc::new(codemode_core::components::auth::oauth::OAuth::new(
            db.clone(),
            "test.example".into(),
        )),
        log: log::Log::new(db.clone()),
        auth,
        sessions: Arc::new(sessions.clone()),
        identity_enabled: false,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        hostname: "test.example".into(),
        workspaces: Arc::new(endpoints),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let task_stop = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state))
            .with_graceful_shutdown(task_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    let request = |method: reqwest::Method, path: &str| {
        client
            .request(method, format!("http://{address}{path}"))
            .header("host", "test.example")
            .header("origin", "https://test.example")
    };
    let response = request(reqwest::Method::GET, "/api/tokens")
        .bearer_auth(&agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["cache-control"], "no-store");
    for (method, path) in [
        (
            reqwest::Method::GET,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/scripts",
        ),
        (
            reqwest::Method::POST,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/scripts",
        ),
        (
            reqwest::Method::GET,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/scripts/00000000-0000-0000-0000-000000000000",
        ),
        (
            reqwest::Method::PUT,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/scripts/00000000-0000-0000-0000-000000000000",
        ),
        (
            reqwest::Method::DELETE,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/scripts/00000000-0000-0000-0000-000000000000",
        ),
        (reqwest::Method::GET, "/api/sessions"),
        (reqwest::Method::GET, "/api/session-activity"),
        (
            reqwest::Method::PUT,
            "/api/workspaces/00000000-0000-0000-0000-000000000000/policies/source",
        ),
        (
            reqwest::Method::GET,
            "/api/sessions/00000000-0000-0000-0000-000000000000",
        ),
        (
            reqwest::Method::PATCH,
            "/api/sessions/00000000-0000-0000-0000-000000000000",
        ),
        (reqwest::Method::GET, "/api/oauth-apps"),
        (reqwest::Method::PUT, "/api/oauth-apps/github"),
        (reqwest::Method::DELETE, "/api/oauth-apps/github"),
    ] {
        let response = request(method, path)
            .bearer_auth(&agent_token)
            .json(
                &serde_json::json!({"pauseId":Uuid::nil(),"decisions":[{"callId":1,"allow":true}]}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = request(reqwest::Method::POST, "/api/session")
        .header("origin", "https://evil.example")
        .json(
            &serde_json::json!({"email":"ursus95@gmail.com","password":"fixture-browser-password"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = request(reqwest::Method::POST, "/api/session")
        .json(&serde_json::json!({"email":"ursus95@gmail.com","password":"wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = request(reqwest::Method::POST, "/api/session")
        .json(&serde_json::json!({"email":"ursus95@gmail.com","password":"fixture-browser-password","role":"owner"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response
            .json::<serde_json::Value>()
            .await
            .unwrap()
            .get("error")
            .is_some()
    );
    let response = request(reqwest::Method::POST, "/api/session")
        .json(
            &serde_json::json!({"email":"ursus95@gmail.com","password":"fixture-browser-password"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response.headers()["set-cookie"].to_str().unwrap();
    for flag in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=43200",
    ] {
        assert!(set_cookie.contains(flag));
    }
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    let session_id:Uuid=sqlx::query_scalar("INSERT INTO agent_sessions(token_id,workspace_id,client) SELECT id,workspace_id,name FROM access_tokens WHERE id=$1 RETURNING id").bind(agent_id).fetch_one(&pool).await.unwrap();
    let execution_id:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,session_id,code,title,result,output,status,tool_calls,finished_at) VALUES($1,$2,'# API history','API history','42','hello','completed',1,now()) RETURNING id").bind(agent_id).bind(session_id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome,preview) VALUES($1,0,'fixture.read','{}','ok','preview')").bind(execution_id).execute(&pool).await.unwrap();
    let response = request(reqwest::Method::GET, "/api/sessions?q=API%20history")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(page["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(page["sessions"][0]["title"], "API history");
    let response = request(reqwest::Method::GET, &format!("/api/sessions/{session_id}"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(detail["executions"][0]["result"], "42");
    assert_eq!(detail["executions"][0]["output"], "hello");
    assert_eq!(detail["executions"][0]["calls"][0]["preview"], "preview");
    let path = format!("/api/sessions/{session_id}");
    let response = request(reqwest::Method::PATCH, &path)
        .header("cookie", &cookie)
        .header("origin", "https://evil.example")
        .json(&serde_json::json!({"title":"wrong origin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for (title, status) in [
        ("", StatusCode::BAD_REQUEST),
        ("Renamed session", StatusCode::NO_CONTENT),
    ] {
        let response = request(reqwest::Method::PATCH, &path)
            .header("cookie", &cookie)
            .json(&serde_json::json!({"title":title}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let response = request(reqwest::Method::GET, "/api/sessions?q=Renamed")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    let renamed = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(renamed["sessions"][0]["title"], "Renamed session");
    assert_eq!(
        renamed["sessions"][0]["expiresAt"],
        page["sessions"][0]["expiresAt"]
    );
    let response = request(
        reqwest::Method::GET,
        &format!("/api/sessions?q={}", "x".repeat(1025)),
    )
    .header("cookie", &cookie)
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = request(reqwest::Method::GET, "/api/tokens")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let tokens = response.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "audience",
            "id",
            "lastUsedAt",
            "name",
            "oauth",
            "resource",
            "revoked",
            "workspaceId"
        ]
    );
    let response=request(reqwest::Method::POST,"/api/sources").header("cookie",&cookie).json(&serde_json::json!({"slug":"fixture","kind":"mcp_http","url":"https://fixture.example/mcp","requiresAccount":true})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response.json::<serde_json::Value>().await.unwrap();
    let source: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let response = request(reqwest::Method::GET, &format!("/api/sources/{source}"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let source_info = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(source_info["pythonName"], "fixture");
    assert_eq!(source_info["status"], "needs_link");
    assert_eq!(source_info["name"], "fixture");
    assert_eq!(source_info["icon"], "plug");
    let path = format!("/api/sources/{source}");
    let updated = request(reqwest::Method::PATCH, &path)
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"Acme Billing","icon":"card"}))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::NO_CONTENT);
    let renamed = request(reqwest::Method::GET, &path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(renamed["name"], "Acme Billing");
    assert_eq!(renamed["icon"], "card");
    assert_eq!(renamed["slug"], "fixture");
    assert_eq!(renamed["pythonName"], "fixture");
    let invalid = request(reqwest::Method::PATCH, &path)
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"Acme Billing","icon":"arbitrary-svg"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let response = request(
        reqwest::Method::POST,
        &format!("/api/sources/{source}/sync"),
    )
    .header("cookie", &cookie)
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = request(reqwest::Method::POST, "/api/workspaces")
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":" ","sourceIds":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = request(reqwest::Method::POST, "/api/workspaces")
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"Work projects","sourceIds":[source],"allSources":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = request(reqwest::Method::POST, "/api/workspaces")
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"Work projects","sourceIds":[source]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let endpoint = response.json::<serde_json::Value>().await.unwrap();
    let endpoint_id = endpoint["id"].as_str().unwrap();
    let endpoint_path = format!("/api/workspaces/{endpoint_id}");
    for (method, path) in [
        (
            reqwest::Method::POST,
            format!("/api/workspaces/{endpoint_id}/scripts"),
        ),
        (
            reqwest::Method::PUT,
            format!("/api/workspaces/{endpoint_id}/scripts/{}", Uuid::nil()),
        ),
        (
            reqwest::Method::DELETE,
            format!("/api/workspaces/{endpoint_id}/scripts/{}", Uuid::nil()),
        ),
    ] {
        let response = request(method, &path)
            .header("cookie", &cookie)
            .header("origin", "https://evil.example")
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    let response = request(reqwest::Method::GET, &endpoint_path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let endpoint = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(endpoint["name"], "Work projects");
    assert!(endpoint.get("hostname").is_none());
    assert_eq!(endpoint["sourceIds"], serde_json::json!([source]));
    assert_eq!(endpoint["allSources"], false);
    let response = request(reqwest::Method::POST, "/api/tokens")
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"work","workspaceId":endpoint_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let token = response.json::<serde_json::Value>().await.unwrap();
    let token = token["token"].as_str().unwrap();
    let auth = Auth::new(db.clone());
    assert!(
        auth.authenticate(token, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        auth.authenticate(token, "work.endpoints.example")
            .await
            .unwrap()
            .is_none()
    );
    for (method, path) in [
        (reqwest::Method::GET, "/api/approvals"),
        (
            reqwest::Method::GET,
            "/api/approvals/00000000-0000-0000-0000-000000000000",
        ),
        (
            reqwest::Method::POST,
            "/api/approvals/00000000-0000-0000-0000-000000000000",
        ),
    ] {
        for signed_in in [false, true] {
            let mut response = request(method.clone(), path);
            if signed_in {
                response = response.header("cookie", &cookie);
            }
            assert_eq!(
                response.send().await.unwrap().status(),
                StatusCode::NOT_FOUND
            );
        }
    }
    let response = request(reqwest::Method::PUT, &endpoint_path)
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"Work projects","sourceIds":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = request(reqwest::Method::GET, "/api/workspaces")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    let endpoints = response.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(endpoints.len(), 2);
    assert!(
        endpoints.iter().find(|e| e["id"] == endpoint_id).unwrap()["sourceIds"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let response = request(reqwest::Method::DELETE, &endpoint_path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        auth.authenticate(token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    let response = request(reqwest::Method::GET, &endpoint_path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let path = format!("/api/accounts/{source}");
    let response = request(reqwest::Method::PUT, &path)
        .header("cookie", &cookie)
        .json(&serde_json::json!({"credential":"fixture-account-secret"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = request(reqwest::Method::GET, "/api/accounts")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    let accounts = response.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(
        accounts[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["expiresAt", "keyId", "kind", "sourceId"]
    );
    let response = request(reqwest::Method::GET, "/api/executions")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let executions = response.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0]["id"], execution_id.to_string());
    let response = request(reqwest::Method::POST, "/api/tokens")
        .header("cookie", &cookie)
        .json(&serde_json::json!({"name":"browser-created","administration":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response.json::<serde_json::Value>().await.unwrap();
    let token_id = created["id"].as_str().unwrap();
    assert!(created["token"].as_str().unwrap().starts_with("cm_"));
    let response = request(reqwest::Method::DELETE, &format!("/api/tokens/{token_id}"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id=$1")
            .bind(token_id.parse::<Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked);
    let response = request(reqwest::Method::DELETE, &path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = request(reqwest::Method::POST, "/api/session")
        .header("cookie", &cookie)
        .json(
            &serde_json::json!({"email":"ursus95@gmail.com","password":"fixture-browser-password"}),
        )
        .send()
        .await
        .unwrap();
    let rotated = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_ne!(cookie, rotated);
    let response = request(reqwest::Method::DELETE, "/api/session")
        .header("cookie", &rotated)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = request(reqwest::Method::GET, "/api/tokens")
        .header("cookie", &rotated)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = request(reqwest::Method::GET, "/api/tokens")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = request(reqwest::Method::DELETE, "/api/session")
        .header("cookie", &rotated)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["set-cookie"], CLEAR_COOKIE);
    let response = request(reqwest::Method::GET, "/api/tokens")
        .header("cookie", rotated)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    stop.cancel();
    server.await.unwrap();
}

async fn identity_request(
    app: axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    origin: Option<&str>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "test.example")
        .header("cookie", cookie);
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if method == "POST" {
        request = request.header("content-type", "application/json");
    }
    app.oneshot(
        request
            .body(axum::body::Body::from(if method == "POST" {
                r#"{"email":"new@olegpustovit.com","password":"fixture-browser-password"}"#
            } else {
                ""
            }))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../core/migrations")]
async fn identity_browser_mode_closes_password_routes_and_isolates_cookies_and_logout(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let auth = Auth::new(db.clone());
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let endpoints =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    endpoints.initialize().await.unwrap();
    let sources = Sources::new(
        db.clone(),
        codemode_core::components::catalogue::Catalogue::new(db.clone()),
        accounts.clone(),
        endpoints.clone(),
    );
    let sessions = Sessions::new(db.clone());
    let legacy = sessions
        .register("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap();
    let state = State {
        agent_sessions: codemode_core::components::agent_sessions::AgentSessions::new(db.clone()),
        scripts: codemode_core::components::scripts::Scripts::new(db.clone()),
        oauth: Arc::new(codemode_core::components::auth::oauth::OAuth::new(
            db.clone(),
            "test.example".into(),
        )),
        log: log::Log::new(db.clone()),
        auth,
        sessions: Arc::new(sessions.clone()),
        identity_enabled: true,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        hostname: "test.example".into(),
        workspaces: Arc::new(endpoints),
    };

    sessions
        .link_identity("browser-subject", "ursus95@gmail.com")
        .await
        .unwrap();
    let id = sessions
        .create_identity_session("browser-subject")
        .await
        .unwrap()
        .unwrap();
    use secrecy::ExposeSecret;
    let legacy_cookie = format!("__Host-codemode_session={}", legacy.expose_secret());
    let id_cookie = format!("__Host-codemode_id_session={}", id.expose_secret());
    let app = router(state.clone());
    let response = identity_request(app.clone(), "GET", "/api/session", &id_cookie, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(metadata["authenticated"], true);
    assert_eq!(metadata["firstUser"], false);
    assert_eq!(metadata["user"].as_object().unwrap().len(), 4);
    assert_eq!(
        identity_request(app.clone(), "GET", "/api/sources", &id_cookie, None)
            .await
            .status(),
        StatusCode::OK
    );
    for invalid in [
        legacy_cookie.clone(),
        format!("__Host-codemode_id_session={}", legacy.expose_secret()),
        format!("{id_cookie}; {id_cookie}"),
    ] {
        assert_eq!(
            identity_request(app.clone(), "GET", "/api/sources", &invalid, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let rollback = router(State {
        identity_enabled: false,
        ..state
    });
    for invalid in [
        id_cookie.clone(),
        format!("__Host-codemode_session={}", id.expose_secret()),
    ] {
        assert_eq!(
            identity_request(rollback.clone(), "GET", "/api/sources", &invalid, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    for path in ["/api/session", "/api/register"] {
        let response =
            identity_request(app.clone(), "POST", path, "", Some("https://test.example")).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("sign in with Microapps ID required"),
            "{path}"
        );
    }
    for (router, id_enabled) in [(app.clone(), true), (rollback.clone(), false)] {
        let response = identity_request(router, "GET", "/api/config", "", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let config: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(config["idEnabled"], id_enabled);
        assert_eq!(config["loginEnabled"], !id_enabled);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        users
    );
    assert_eq!(
        identity_request(
            app.clone(),
            "DELETE",
            "/api/session",
            &id_cookie,
            Some("https://evil.example")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert!(
        sessions
            .identity_user(id.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    let response = identity_request(
        app,
        "DELETE",
        "/api/session",
        &id_cookie,
        Some("https://test.example"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-codemode_id_session=;")
    );
    assert!(
        sessions
            .identity_user(id.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .user(legacy.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        identity_request(rollback, "GET", "/api/sources", &legacy_cookie, None)
            .await
            .status(),
        StatusCode::OK
    );
}
