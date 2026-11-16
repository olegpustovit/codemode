use super::{
    endpoints::{rpc, tool_result},
    server,
};
use codemode_core::{
    components::{
        accounts::Accounts,
        auth::{Auth, oauth::OAuth},
        catalogue::Catalogue,
        execution::Execution,
        sessions::Sessions,
        sources::Sources,
        workspaces::Workspaces,
    },
    infrastructure::{postgres::Database, sandbox::MontySandbox},
};
use sqlx::ConnectOptions;
use std::{sync::Arc, time::Duration};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}

#[sqlx::test(migrations = "../core/migrations")]
async fn oauth_browser_consent_issues_tokens_for_both_mcp_eras_and_refresh_reuse_revokes_them(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let workspace = endpoints.create("work", &[]).await.unwrap();
    let auth = Auth::new(db.clone());
    let (_, agent) = auth.create("agent", "test.example").await.unwrap();
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(
        db.clone(),
        catalogue.clone(),
        accounts.clone(),
        endpoints.clone(),
    );
    let worker = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/monty/bin/monty")
        });
    let sandbox = Arc::new(
        MontySandbox::new(&worker, Duration::from_secs(1), 32768)
            .await
            .unwrap(),
    );
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let execution = Arc::new(Execution::new(
        db.clone(),
        sources.clone(),
        sandbox.clone(),
        tasks.clone(),
        stop.clone(),
        Default::default(),
    ));
    let sessions = Sessions::new(db.clone());
    sessions
        .register("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap();
    let state = server::State {
        mcp_tasks: server::Tasks::default(),
        rate_limit: Arc::new(
            codemode_core::components::auth::rate_limit::RateLimiter::new(60, 600),
        ),
        db: db.clone(),
        auth,
        catalogue,
        execution,
        hostname: "test.example".into(),
        sessions: Arc::new(sessions),
        identity_enabled: false,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        workspaces: Arc::new(endpoints),
        stop: stop.clone(),
        oauth: Arc::new(OAuth::new(db, "test.example".into())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, server::router(state))
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let url = format!("http://{address}");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let request = |method: reqwest::Method, path: &str| {
        client
            .request(method, format!("{url}{path}"))
            .header("host", "test.example")
    };
    let response = request(
        reqwest::Method::GET,
        "/.well-known/oauth-authorization-server",
    )
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let metadata = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(metadata["issuer"], "https://test.example");
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
    assert_eq!(metadata["client_id_metadata_document_supported"], true);
    let response = request(reqwest::Method::GET, "/oauth/client-metadata.json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let document = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        document["client_id"],
        "https://test.example/oauth/client-metadata.json"
    );
    assert_eq!(
        document["redirect_uris"],
        serde_json::json!(["https://test.example/api/accounts/oauth/callback"])
    );
    assert_eq!(document["token_endpoint_auth_method"], "none");
    assert!(document.get("client_secret").is_none());
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource/mcp",
        "/api/config",
        "/mcp",
    ] {
        let response = request(reqwest::Method::GET, path)
            .header("host", "work.endpoints.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }
    let response = client
        .get(format!("{url}/.well-known/oauth-authorization-server"))
        .header("host", "work.endpoints.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let response = request(reqwest::Method::OPTIONS, "/oauth/token")
        .header("origin", "https://client.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    let response = request(reqwest::Method::POST, "/oauth/register")
        .json(&serde_json::json!({"redirect_uris":["http://evil.example/callback"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response=request(reqwest::Method::POST,"/oauth/register").json(&serde_json::json!({"client_name":"Fixture agent","redirect_uris":["https://client.example/callback"],"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"]})).send().await.unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let registered = response.json::<serde_json::Value>().await.unwrap();
    assert!(registered.get("client_secret").is_none());
    let id = registered["client_id"].as_str().unwrap();
    let authorization = serde_json::json!({"response_type":"code","client_id":id,"redirect_uri":"https://client.example/callback","resource":"https://test.example/mcp","code_challenge":"E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM","code_challenge_method":"S256","state":"client-state","scope":"mcp"});
    let query = form(
        &authorization
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
            .collect::<Vec<_>>(),
    );
    let response = request(reqwest::Method::GET, &format!("/oauth/authorize?{query}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    assert_eq!(
        response.headers()["location"],
        format!("/authorize?{query}")
    );
    let consent_path = format!("/api/oauth/authorize?{query}");
    let response = request(reqwest::Method::GET, &consent_path)
        .bearer_auth(&agent)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let response = request(reqwest::Method::POST, "/api/session")
        .header("origin", "https://test.example")
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
    let denied_registry = request(reqwest::Method::GET, "/api/integrations")
        .send()
        .await
        .unwrap();
    assert_eq!(denied_registry.status(), 401);
    let registry = request(reqwest::Method::GET, "/api/integrations")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(registry.status(), 200);
    let registry = registry.json::<serde_json::Value>().await.unwrap();
    assert!(
        registry
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["slug"] == "github" && entry["methods"][0] == "api_key")
    );
    let missing = request(reqwest::Method::POST, "/api/integrations/not-a-provider")
        .header("cookie", &cookie)
        .header("origin", "https://test.example")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let response = request(reqwest::Method::GET, &consent_path)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let consent = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(consent["clientName"], "Fixture agent");
    assert_eq!(consent["resource"], "https://test.example/mcp");
    let mut decision = authorization.clone();
    decision["allow"] = serde_json::json!(true);
    decision["workspaceId"] = serde_json::json!(workspace);
    let response = request(reqwest::Method::POST, "/api/oauth/authorize")
        .bearer_auth(&agent)
        .header("origin", "https://test.example")
        .json(&decision)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let response = request(reqwest::Method::POST, "/api/oauth/authorize")
        .header("cookie", &cookie)
        .header("origin", "https://evil.example")
        .json(&decision)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    decision["allow"] = serde_json::json!(false);
    let response = request(reqwest::Method::POST, "/api/oauth/authorize")
        .header("cookie", &cookie)
        .header("origin", "https://test.example")
        .json(&decision)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let denied = response.json::<serde_json::Value>().await.unwrap();
    let denied = url::Url::parse(denied["redirectUri"].as_str().unwrap()).unwrap();
    assert!(
        denied
            .query_pairs()
            .any(|(k, v)| k == "error" && v == "access_denied")
    );
    assert!(!denied.query_pairs().any(|(k, _)| k == "code"));
    decision["allow"] = serde_json::json!(true);
    decision["workspaceId"] = serde_json::json!(workspace);
    let response = request(reqwest::Method::POST, "/api/oauth/authorize")
        .header("cookie", &cookie)
        .header("origin", "https://test.example")
        .json(&decision)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let approved = response.json::<serde_json::Value>().await.unwrap();
    let redirect = url::Url::parse(approved["redirectUri"].as_str().unwrap()).unwrap();
    assert!(
        redirect
            .query_pairs()
            .any(|(k, v)| k == "state" && v == "client-state")
    );
    assert!(
        redirect
            .query_pairs()
            .any(|(k, v)| k == "iss" && v == "https://test.example")
    );
    let code = redirect
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let body = form(&[
        ("client_id", id),
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", "https://client.example/callback"),
        ("resource", "https://test.example/mcp"),
        (
            "code_verifier",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        ),
    ]);
    let response = request(reqwest::Method::POST, "/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let tokens = response.json::<serde_json::Value>().await.unwrap();
    let token = tokens["access_token"].as_str().unwrap();
    for version in ["2025-11-25", "2026-07-28"] {
        if version == "2025-11-25" {
            rpc(&client,&url,"test.example",token,version,"initialize",serde_json::json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"oauth-test","version":"1"}})).await;
        }
        let result = tool_result(
            rpc(
                &client,
                &url,
                "test.example",
                token,
                version,
                "tools/call",
                serde_json::json!({"name":"execute","arguments":{"code":"return 42"}}),
            )
            .await,
        );
        assert_eq!(result["status"], "completed");
        assert_eq!(result["result"], 42);
    }
    let body = form(&[
        ("client_id", id),
        ("grant_type", "refresh_token"),
        ("resource", "https://test.example/mcp"),
        ("refresh_token", tokens["refresh_token"].as_str().unwrap()),
    ]);
    let response = request(reqwest::Method::POST, "/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let fresh = response.json::<serde_json::Value>().await.unwrap();
    let response = request(reqwest::Method::POST, "/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response = client
        .post(format!("{url}/mcp"))
        .header("host", "test.example")
        .bearer_auth(fresh["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    // Separate OAuth resource, with no workspace selection or global administrator bypass.
    let mut management = authorization.clone();
    management["resource"] = serde_json::json!("https://test.example/mcp-admin");
    management["scope"] = serde_json::json!("admin");
    management["allow"] = serde_json::json!(true);
    let approved = request(reqwest::Method::POST, "/api/oauth/authorize")
        .header("cookie", &cookie)
        .header("origin", "https://test.example")
        .json(&management)
        .send()
        .await
        .unwrap();
    assert_eq!(approved.status(), 200);
    let approved = approved.json::<serde_json::Value>().await.unwrap();
    let redirect = url::Url::parse(approved["redirectUri"].as_str().unwrap()).unwrap();
    let code = redirect
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let body = form(&[
        ("client_id", id),
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", "https://client.example/callback"),
        ("resource", "https://test.example/mcp-admin"),
        (
            "code_verifier",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        ),
    ]);
    let granted = request(reqwest::Method::POST, "/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(granted.status(), 200);
    let granted = granted.json::<serde_json::Value>().await.unwrap();
    assert_eq!(granted["scope"], "admin");
    let management_token = granted["access_token"].as_str().unwrap();
    for (path, bearer) in [("/mcp", management_token), ("/mcp-admin", agent.as_str())] {
        let denied = request(reqwest::Method::POST, path)
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);
        assert!(
            denied.headers()["www-authenticate"]
                .to_str()
                .unwrap()
                .contains(&format!("oauth-protected-resource{path}"))
        );
    }
    let metadata = request(
        reqwest::Method::GET,
        "/.well-known/oauth-protected-resource/mcp-admin",
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(metadata["resource"], "https://test.example/mcp-admin");
    assert_eq!(metadata["scopes_supported"], serde_json::json!(["admin"]));
    let admin_rpc = |token: String, method: &str, params: serde_json::Value| {
        request(reqwest::Method::POST, "/mcp-admin")
            .bearer_auth(token)
            .header("accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
    };
    let list = admin_rpc(management_token.into(), "tools/list", serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 10);
    let invoke = |token: String, name: &str, args: serde_json::Value| {
        admin_rpc(
            token,
            "tools/call",
            serde_json::json!({"name":name,"arguments":args}),
        )
    };
    let registry = invoke(
        management_token.into(),
        "list_integrations",
        serde_json::json!({}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    let registry = tool_result(registry["result"].clone());
    assert!(
        registry
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["slug"] == "github" && entry["sourceId"].is_null())
    );
    let saved=invoke(management_token.into(),"save_source",serde_json::json!({"source":{"slug":"admin-private","kind":"mcp_http","url":"https://fixture.example/mcp","requiresAccount":true}})).send().await.unwrap().json::<serde_json::Value>().await.unwrap();
    let source = tool_result(saved["result"].clone());
    let source_id = source.as_str().unwrap();
    let saved = invoke(
        management_token.into(),
        "save_workspace",
        serde_json::json!({"name":"Admin managed workspace","sourceIds":[source_id]}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    let saved = tool_result(saved["result"].clone());
    let saved_id = saved.as_str().unwrap();
    let list = invoke(
        management_token.into(),
        "list_sources",
        serde_json::json!({}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    let list = tool_result(list["result"].clone());
    assert_eq!(list[0]["id"], source);
    for hidden in ["url", "config", "credential", "bearer_env"] {
        assert!(list[0].get(hidden).is_none());
    }
    let bad = invoke(
        management_token.into(),
        "save_source",
        serde_json::json!({"source":{"slug":"unsafe","kind":"mcp_stdio","command":"/bin/sh"}}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(bad["result"]["isError"], true);
    let connection = invoke(
        management_token.into(),
        "source_connection_url",
        serde_json::json!({"id":source_id}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(
        tool_result(connection["result"].clone())["url"],
        format!("https://test.example/sources/{source_id}/accounts")
    );
    // A later user has the same management surface, restricted by immutable tenant RLS.
    let second=request(reqwest::Method::POST,"/api/register").header("origin","https://test.example")
        .json(&serde_json::json!({"email":"admin-check@olegpustovit.com","password":"fixture-browser-password"})).send().await.unwrap();
    assert!(second.status().is_success());
    let second_cookie = second.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let second = request(reqwest::Method::POST, "/api/tokens")
        .header("origin", "https://test.example")
        .header("cookie", second_cookie)
        .json(&serde_json::json!({"name":"second management","administration":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 201);
    let second = second.json::<serde_json::Value>().await.unwrap();
    let second_token = second["token"].as_str().unwrap();
    let list = invoke(second_token.into(), "list_sources", serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(tool_result(list["result"].clone()), serde_json::json!([]));
    let bad = invoke(
        second_token.into(),
        "save_workspace",
        serde_json::json!({"id":saved_id,"name":"Stolen","sourceIds":[]}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(bad["result"]["isError"], true);
    let bad = invoke(
        second_token.into(),
        "source_connection_url",
        serde_json::json!({"id":source_id}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(bad["result"]["isError"], true);
    let removed = invoke(
        management_token.into(),
        "delete_workspace",
        serde_json::json!({"id":saved_id}),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_ne!(removed["result"]["isError"], true);
    let rows: Vec<(String, serde_json::Value, String)> = sqlx::query_as(
        "SELECT tool,args_redacted,outcome FROM administration_calls ORDER BY started_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 10);
    assert!(rows.iter().all(|(_, _, outcome)| outcome != "running"));
    let args = &rows
        .iter()
        .find(|(tool, _, _)| tool == "save_source")
        .unwrap()
        .1;
    assert_eq!(args["source"], "[redacted]");
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}
