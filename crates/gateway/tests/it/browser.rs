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
    let (_, agent_token) = auth.create("agent", "test.example").await.unwrap();
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
        scripts: codemode_core::components::scripts::Scripts::new(db.clone()),
        approvals: codemode_core::components::execution::approvals::Approvals::new(
            db.clone(),
            [73; 32],
        ),
        oauth: Arc::new(codemode_core::components::auth::oauth::OAuth::new(
            db.clone(),
            "test.example".into(),
        )),
        log: log::Log::new(db.clone()),
        auth,
        sessions: Arc::new(sessions.clone()),
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
            "name",
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
    assert_eq!(source_info["status"], "pending");
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
    let response = request(
        reqwest::Method::POST,
        "/api/approvals/00000000-0000-0000-0000-000000000000",
    )
    .header("cookie", &cookie)
    .header("origin", "https://evil.example")
    .json(&serde_json::json!({"pauseId":Uuid::nil(),"decisions":[{"callId":1,"allow":true}]}))
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = request(reqwest::Method::GET, "/api/approvals")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!([])
    );
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
    assert!(
        response
            .json::<Vec<serde_json::Value>>()
            .await
            .unwrap()
            .is_empty()
    );
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
