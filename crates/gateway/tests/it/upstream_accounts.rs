use super::admin;
#[path = "../../../core/tests/support/oauth_provider.rs"]
mod fixture;
use axum::{Router, routing::get};
use codemode_core::{
    components::{
        accounts::Accounts,
        auth::{Auth, oauth::OAuth},
        catalogue::Catalogue,
        execution::log::Log,
        sessions::Sessions,
        sources::Sources,
        workspaces::Workspaces,
    },
    infrastructure::postgres::Database,
};
use sqlx::ConnectOptions;
use std::sync::{Arc, atomic::Ordering};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[sqlx::test(migrations = "../core/migrations")]
async fn browser_upstream_linking_requires_its_session_and_exposes_only_account_metadata(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let sources = Sources::new(
        db.clone(),
        Catalogue::new(db.clone()),
        accounts.clone(),
        endpoints.clone(),
    );
    let auth = Auth::new(db.clone());
    let (_, agent) = auth.create("agent", "test.example").await.unwrap();
    let sessions = Sessions::new(db.clone());
    sessions
        .register("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap();
    let state = admin::State {
        access: Default::default(),
        agent_sessions: codemode_core::components::agent_sessions::AgentSessions::new(db.clone()),
        scripts: codemode_core::components::scripts::Scripts::new(db.clone()),
        log: Log::new(db.clone()),
        auth,
        sessions: Arc::new(sessions.clone()),
        identity_enabled: false,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        hostname: "test.example".into(),
        workspaces: Arc::new(endpoints),
        oauth: Arc::new(OAuth::new(db, "test.example".into())),
    };
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider =
        fixture::Provider::new(format!("http://{}", upstream.local_addr().unwrap()), true);
    let challenge = provider.clone();
    let provider_app = provider.routes().merge(Router::new().route(
        "/mcp",
        get(move || {
            let provider = challenge.clone();
            async move { provider.challenge() }
        }),
    ));
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let upstream_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(upstream, provider_app)
            .with_graceful_shutdown(upstream_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, admin::router(state))
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let request = |method: reqwest::Method, path: &str| {
        client
            .request(method, format!("http://{address}{path}"))
            .header("host", "test.example")
            .header("origin", "https://test.example")
    };
    let response = request(reqwest::Method::POST, "/api/session")
        .json(
            &serde_json::json!({"email":"ursus95@gmail.com","password":"fixture-browser-password"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let app = serde_json::json!({"clientId":"browser-app-id","clientSecret":"browser-app-secret"});
    assert_eq!(
        request(reqwest::Method::GET, "/api/oauth-apps")
            .bearer_auth(&agent)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        request(reqwest::Method::PUT, "/api/oauth-apps/github")
            .header("cookie", &cookie)
            .header("origin", "https://evil.example")
            .json(&app)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    for (provider, body, status) in [
        ("linear", app.clone(), 400),
        (
            "github",
            serde_json::json!({"clientId":" ","clientSecret":"secret"}),
            400,
        ),
        (
            "github",
            serde_json::json!({"clientId":"id","clientSecret":"secret","extra":"unexpected"}),
            422,
        ),
    ] {
        assert_eq!(
            request(reqwest::Method::PUT, &format!("/api/oauth-apps/{provider}"))
                .header("cookie", &cookie)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            status
        );
    }
    assert_eq!(
        request(reqwest::Method::PUT, "/api/oauth-apps/github")
            .header("cookie", &cookie)
            .json(&app)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let response = request(reqwest::Method::GET, "/api/oauth-apps")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), r#"["github"]"#);
    assert_eq!(
        request(reqwest::Method::DELETE, "/api/oauth-apps/github")
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let response=request(reqwest::Method::POST,"/api/sources").header("cookie",&cookie).json(&serde_json::json!({"slug":"provider","kind":"mcp_http","url":format!("{}/mcp",provider.origin),"requiresAccount":true})).send().await.unwrap();
    assert_eq!(response.status(), 201);
    let created = response.json::<serde_json::Value>().await.unwrap();
    let id = created["id"].as_str().unwrap();
    let path = format!("/api/accounts/{id}/oauth");
    let response = request(reqwest::Method::POST, &path)
        .bearer_auth(&agent)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let response = request(reqwest::Method::POST, &path)
        .header("cookie", &cookie)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let response = request(reqwest::Method::POST, &path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let begin = response.json::<serde_json::Value>().await.unwrap();
    let csrf = provider.prepare(begin["authorizationUrl"].as_str().unwrap());
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("state", csrf.as_str()),
            ("code", "fixture-code"),
            ("iss", provider.origin.as_str()),
        ])
        .finish();
    let callback = format!("/api/accounts/oauth/callback?{query}");
    let response = request(reqwest::Method::GET, &callback)
        .bearer_auth(&agent)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let response = client
        .get(format!("http://{address}{callback}"))
        .header("host", "test.example")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    assert_eq!(
        response.headers()["location"],
        format!("/integrations?selected={id}&detail=open&oauth=linked")
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let response = request(reqwest::Method::GET, "/api/accounts")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    let accounts = response.text().await.unwrap();
    for secret in [
        "provider-access-1",
        "provider-refresh-1",
        "fixture-client-secret",
        "pkce_verifier",
        "ciphertext",
    ] {
        assert!(!accounts.contains(secret));
    }
    let accounts: Vec<serde_json::Value> = serde_json::from_str(&accounts).unwrap();
    assert_eq!(accounts[0]["kind"], "oauth");
    let response = request(reqwest::Method::GET, &callback)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response = request(reqwest::Method::POST, &path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    let begin = response.json::<serde_json::Value>().await.unwrap();
    let csrf = provider.prepare(begin["authorizationUrl"].as_str().unwrap());
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("state", csrf.as_str()),
            ("iss", provider.origin.as_str()),
            ("error", "access_denied"),
            (
                "error_description",
                format!("workspace policy rejected access {csrf}\ntry another app").as_str(),
            ),
        ])
        .finish();
    let callback = format!("/api/accounts/oauth/callback?{query}");
    let response = request(reqwest::Method::GET, &callback)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    let location = url::Url::parse(&format!(
        "https://test.example{}",
        response.headers()["location"].to_str().unwrap()
    ))
    .unwrap();
    let query: std::collections::HashMap<_, _> = location.query_pairs().into_owned().collect();
    assert_eq!(query["oauth"], "denied");
    assert_eq!(query["selected"], id);
    let detail = &query["oauthError"];
    assert!(detail.contains("step 5") && detail.contains("workspace policy rejected access"));
    assert!(!detail.contains(&csrf) && !detail.contains('\n'));
    assert_eq!(
        request(reqwest::Method::GET, &callback)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(provider.exchanges.load(Ordering::SeqCst), 1);
    stop.cancel();
    tasks.close();
    tasks.wait().await;
}
