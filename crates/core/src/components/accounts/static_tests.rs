use super::*;
#[path = "../../../tests/support/oauth_provider.rs"]
mod fixture;

#[tokio::test]
async fn static_app_exchanges_pkce_code_with_secret_post_without_client_registration() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let fixture = fixture::Provider::new(origin.clone(), false);
    fixture
        .static_app
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let stop = tokio_util::sync::CancellationToken::new();
    let tasks = tokio_util::task::TaskTracker::new();
    let app = fixture.routes();
    let server_stop = stop.clone();
    tasks.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
            .unwrap()
    });
    let metadata=serde_json::from_value(serde_json::json!({"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"issuer":origin,"scopes_supported":["tools","offline_access"],"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["client_secret_post"]})).unwrap();
    let provider = Provider {
        url: format!("{origin}/mcp"),
        resource: format!("{origin}/mcp"),
        metadata,
        client_id: "fixture-client".into(),
        client_secret: Some(fixture::CLIENT_SECRET.into()),
        redirect_uri: "https://test.example/api/accounts/oauth/callback".into(),
    };
    let (url, state, pending) = provider.authorize().await.unwrap();
    assert_eq!(fixture.prepare(&url), state);
    let store = InMemoryCredentialStore::new();
    let mut manager = pending.provider.manager(store.clone()).await.unwrap();
    let states = InMemoryStateStore::new();
    states.save(&state, pending.authorization).await.unwrap();
    manager.set_state_store(states);
    manager
        .exchange_code_for_token_with_issuer("fixture-code", &state, Some(&origin))
        .await
        .unwrap();
    let credentials = store.load().await.unwrap().unwrap();
    validate_credentials(&credentials).unwrap();
    assert_eq!(
        fixture.exchanges.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        fixture
            .registrations
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    stop.cancel();
    tasks.close();
    tasks.wait().await;
}
