use super::identity;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    routing::get,
};
use codemode_core::{components::sessions::Sessions, infrastructure::postgres::Database};
use idclient::{AppError, Application, Claims};
use secrecy::ExposeSecret;
use sqlx::ConnectOptions;
use tower::ServiceExt;

#[sqlx::test(migrations = "../core/migrations")]
async fn identity_sdk_adapter_rotates_only_id_sessions_and_clears_only_its_cookie(
    pool: sqlx::PgPool,
) {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    let sessions = Sessions::new(db);
    let app = identity::App::new(sessions.clone());
    let claims = |verified| Claims {
        subject: "sdk-member".into(),
        email: "member@olegpustovit.com".into(),
        email_verified: verified,
        name: "Untrusted display name".into(),
        picture: None,
        amr: vec![],
    };
    assert!(matches!(
        app.resolve_user(claims(false)).await,
        Err(AppError::Denied)
    ));
    let subject = app.resolve_user(claims(true)).await.unwrap();
    let first = sessions
        .create_identity_session(&subject)
        .await
        .unwrap()
        .unwrap();
    let request = Request::builder()
        .header(
            header::COOKIE,
            format!("__Host-codemode_id_session={}", first.expose_secret()),
        )
        .body(())
        .unwrap()
        .into_parts()
        .0;
    sqlx::query("CREATE FUNCTION reject_session_rotation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected session rotation failure'; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_session_rotation BEFORE DELETE ON identity_sessions FOR EACH ROW EXECUTE FUNCTION reject_session_rotation()").execute(&pool).await.unwrap();
    assert!(matches!(
        app.create_session(subject.clone(), request).await,
        Err(AppError::Unavailable)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(
        sessions
            .identity_user(first.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("DROP TRIGGER reject_session_rotation ON identity_sessions")
        .execute(&pool)
        .await
        .unwrap();
    let request = Request::builder()
        .header(
            header::COOKIE,
            format!("__Host-codemode_id_session={}", first.expose_secret()),
        )
        .body(())
        .unwrap()
        .into_parts()
        .0;
    let headers = app.create_session(subject, request).await.unwrap();
    assert_eq!(headers.len(), 1);
    let cookie = headers[0].to_str().unwrap();
    assert!(cookie.starts_with("__Host-codemode_id_session="));
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=43200",
    ] {
        assert!(cookie.contains(attribute));
    }
    assert!(!cookie.contains("Domain="));
    assert!(
        sessions
            .identity_user(first.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    let mut map = HeaderMap::new();
    map.insert(
        header::COOKIE,
        HeaderValue::from_str(cookie.split(';').next().unwrap()).unwrap(),
    );
    let token = super::admin::cookie(&map, true).unwrap();
    let user = sessions.identity_user(token).await.unwrap().unwrap();
    assert!(!user.admin);
    assert_ne!(user.tenant_id, uuid::Uuid::nil());
    let request = Request::builder()
        .header(header::COOKIE, map[header::COOKIE].clone())
        .body(())
        .unwrap()
        .into_parts()
        .0;
    let cleared = app.clear_session(request).await.unwrap();
    assert_eq!(cleared, [super::admin::CLEAR_ID_COOKIE]);
    assert!(sessions.identity_user(token).await.unwrap().is_none());
}

#[sqlx::test(migrations = "../core/migrations")]
async fn identity_sdk_routes_discover_apply_host_origin_and_return_path_guards(pool: sqlx::PgPool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let metadata = serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["client_secret_basic"],"code_challenge_methods_supported":["S256"]});
    let provider = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || async move { axum::Json(metadata) }),
        )
        .route(
            "/jwks",
            get(|| async { axum::Json(serde_json::json!({"keys":[]})) }),
        );
    let serving = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let sessions = Sessions::new(
        Database::connect(pool.connect_options().to_url_lossy().as_str())
            .await
            .unwrap(),
    );
    let config = idclient::Config {
        issuer: issuer.clone(),
        client_id: "sdk-client".into(),
        client_secret: Some("fixture-client-secret".into()),
        redirect_uri: "https://test.example/auth/callback".into(),
        cookie_key: Box::new([71; 32]).into(),
    };
    let app = identity::router(config, sessions, "test.example".into(), Default::default())
        .await
        .unwrap();
    let send = |method: &str, path: &str, host: &str, origin: Option<&str>| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        app.clone().oneshot(request.body(Body::empty()).unwrap())
    };
    assert_eq!(
        send("GET", "/auth/login", "evil.example", None)
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            "GET",
            "/auth/login?return_to=https%3A%2F%2Fevil.example",
            "test.example",
            None
        )
        .await
        .unwrap()
        .status(),
        StatusCode::BAD_REQUEST
    );
    let response = send(
        "GET",
        "/auth/login?return_to=%2Foauth%2Fauthorize%3Fstate%3Dkeep",
        "test.example",
        None,
    )
    .await
    .unwrap();
    assert!(response.status().is_redirection());
    let location = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    assert_eq!(location.origin().ascii_serialization(), issuer);
    let query: std::collections::HashMap<_, _> = location.query_pairs().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["redirect_uri"], "https://test.example/auth/callback");
    assert!(query.contains_key("state") && query.contains_key("nonce"));
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("__Host-idclient-flow=")
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        send("POST", "/auth/logout", "test.example", None)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            "POST",
            "/auth/logout",
            "test.example",
            Some("https://evil.example")
        )
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
    let response = send(
        "POST",
        "/auth/logout",
        "test.example",
        Some("https://test.example"),
    )
    .await
    .unwrap();
    assert!(response.status().is_redirection());
    assert_eq!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .count(),
        2
    );
    assert_eq!(response.headers()[header::LOCATION], "/");
    let response = send(
        "GET",
        "/auth/callback?code=unverified&state=bad",
        "test.example",
        None,
    )
    .await
    .unwrap();
    assert!(!response.status().is_success());
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    serving.abort();
    let _ = serving.await;
}

#[tokio::test]
async fn identity_startup_configuration_defaults_off_and_never_prints_secrets() {
    let run = |enabled: Option<&str>| {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_codemode"));
        command
            .env_clear()
            .env("CODEMODE_ID_CLIENT_SECRET", "credential-must-not-appear")
            .env("CODEMODE_ID_COOKIE_KEY", "invalid-unused-key");
        if let Some(enabled) = enabled {
            command.env("CODEMODE_ID_ENABLED", enabled);
        }
        command.output()
    };
    for enabled in [None, Some("false"), Some("true"), Some("invalid")] {
        let output = run(enabled).await.unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(
            !message.contains("credential-must-not-appear")
                && !message.contains("invalid-unused-key")
        );
        let expected = match enabled {
            Some("true") => "CODEMODE_ID_CLIENT_ID is required",
            Some("invalid") => "CODEMODE_ID_ENABLED must be true or false",
            _ => "CODEMODE_DATABASE_URL is required",
        };
        assert!(message.contains(expected), "{message}");
    }
}

#[sqlx::test(migrations = "../core/migrations")]
async fn identity_discovery_failure_exits_before_workers_and_source_import(pool: sqlx::PgPool) {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_codemode"))
        .env_clear()
        .env(
            "CODEMODE_DATABASE_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env("CODEMODE_ID_ENABLED", "true")
        .env("CODEMODE_ID_ISSUER", "http://127.0.0.1:0")
        .env("CODEMODE_ID_CLIENT_ID", "startup-client")
        .env(
            "CODEMODE_ID_CLIENT_SECRET",
            "startup-secret-must-not-appear",
        )
        .env("CODEMODE_ID_COOKIE_KEY", "71".repeat(32))
        .env(
            "CODEMODE_SOURCES_FILE",
            "/nonexistent-identity-startup-sources.toml",
        )
        .env("CODEMODE_MONTY_BIN", "/nonexistent-identity-startup-worker")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("configure Microapps ID login and discover issuer"),
        "{combined}"
    );
    assert!(!combined.contains("startup-secret-must-not-appear"));
    assert!(
        !combined.contains("import initial sources")
            && !combined.contains("start Monty worker pool")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
