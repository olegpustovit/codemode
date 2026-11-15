use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use codemode_core::components::{
    auth::oauth::{
        AuthMethod, AuthorizationRequest, ClientAuthentication, ClientMetadata,
        Error as OAuthError, OAuth, TokenRequest,
    },
    workspaces::Workspaces,
};

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
fn metadata(method: AuthMethod) -> ClientMetadata {
    ClientMetadata {
        client_name: "Fixture client".into(),
        redirect_uris: vec!["https://client.example/callback".into()],
        token_endpoint_auth_method: method,
        grant_types: vec!["authorization_code".into(), "refresh_token".into()],
        response_types: vec!["code".into()],
    }
}
async fn setup(
    pool: &sqlx::PgPool,
) -> (
    Database,
    OAuth,
    Workspaces,
    ClientAuthentication,
    AuthorizationRequest,
) {
    let db = database(pool).await;
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let workspace = endpoints.create("work", &[]).await.unwrap();
    let oauth = OAuth::new(db.clone(), "test.example".into());
    let registered = oauth.register(metadata(AuthMethod::None)).await.unwrap();
    assert!(registered.client_secret.is_none());
    let auth = ClientAuthentication {
        client_id: registered.client_id.clone(),
        method: AuthMethod::None,
        secret: None,
    };
    let request = AuthorizationRequest {
        workspace_id: Some(workspace),
        response_type: "code".into(),
        client_id: registered.client_id,
        redirect_uri: "https://client.example/callback".into(),
        resource: "https://test.example/mcp".into(),
        code_challenge: CHALLENGE.into(),
        code_challenge_method: "S256".into(),
        state: Some("opaque-client-state".into()),
        scope: Some("mcp".into()),
    };
    (db, oauth, endpoints, auth, request)
}
fn exchange(code: &str) -> TokenRequest {
    TokenRequest {
        grant_type: "authorization_code".into(),
        resource: "https://test.example/mcp".into(),
        code: Some(code.into()),
        redirect_uri: Some("https://client.example/callback".into()),
        code_verifier: Some(VERIFIER.into()),
        refresh_token: None,
        scope: None,
    }
}
fn refresh(token: &str) -> TokenRequest {
    TokenRequest {
        grant_type: "refresh_token".into(),
        resource: "https://test.example/mcp".into(),
        code: None,
        redirect_uri: None,
        code_verifier: None,
        refresh_token: Some(token.into()),
        scope: None,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_requires_workspace_selection_and_resource_scope_cannot_escalate(pool: sqlx::PgPool) {
    let (db, oauth, endpoints, client, mut authorization) = setup(&pool).await;
    let chosen = authorization.workspace_id.unwrap();
    authorization.workspace_id = None;
    assert!(
        oauth
            .consent(&authorization)
            .await
            .unwrap()
            .workspace_id
            .is_none()
    );
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Request)
    ));
    authorization.workspace_id = Some(Uuid::new_v4());
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Target)
    ));
    authorization.workspace_id = Some(chosen);
    let code = oauth.issue_code(&authorization).await.unwrap();
    let mut wrong = exchange(&code);
    wrong.resource = "https://test.example/mcp-admin".into();
    assert!(matches!(
        oauth.token(&client, &wrong).await,
        Err(OAuthError::Grant)
    ));
    let token = oauth.token(&client, &exchange(&code)).await.unwrap();
    let auth = Auth::new(db.clone());
    let principal = auth
        .authenticate(&token.access_token, "test.example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(principal.workspace_id, chosen);
    assert_eq!(principal.resource, "/mcp");
    let mut wrong = refresh(token.refresh_token.as_deref().unwrap());
    wrong.resource = "https://test.example/mcp-admin".into();
    assert!(matches!(
        oauth.token(&client, &wrong).await,
        Err(OAuthError::Grant)
    ));
    authorization.resource = "https://test.example/mcp-admin".into();
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Scope)
    ));
    authorization.scope = Some("admin".into());
    authorization.workspace_id = None;
    let code = oauth.issue_code(&authorization).await.unwrap();
    let mut request = exchange(&code);
    request.resource = authorization.resource.clone();
    let token = oauth.token(&client, &request).await.unwrap();
    assert_eq!(token.scope, "admin");
    let principal = auth
        .authenticate(&token.access_token, "test.example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(principal.resource, "/mcp-admin");
    assert_eq!(
        principal.workspace_id,
        endpoints.default_id().await.unwrap()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_codes_require_s256_bindings_expiry_and_single_use(pool: sqlx::PgPool) {
    let (db, oauth, _, client, mut authorization) = setup(&pool).await;
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
        CHALLENGE
    );
    authorization.code_challenge_method = "plain".into();
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Request)
    ));
    authorization.code_challenge_method = "S256".into();
    authorization.redirect_uri = "https://evil.example/callback".into();
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Request)
    ));
    authorization.redirect_uri = "https://client.example/callback".into();
    authorization.resource = "https://unknown.endpoints.example/mcp".into();
    assert!(matches!(
        oauth.issue_code(&authorization).await,
        Err(OAuthError::Target)
    ));
    authorization.resource = "https://test.example/mcp".into();
    let code = oauth.issue_code(&authorization).await.unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("SELECT code_hash FROM oauth_codes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Sha256::digest(code.as_bytes()).to_vec());
    let mut request = exchange(&code);
    request.code_verifier = Some("x".repeat(43).into());
    assert!(matches!(
        oauth.token(&client, &request).await,
        Err(OAuthError::Grant)
    ));
    request = exchange(&code);
    request.resource = "https://test.example/mcp-admin".into();
    assert!(matches!(
        oauth.token(&client, &request).await,
        Err(OAuthError::Grant)
    ));
    let tokens = oauth.token(&client, &exchange(&code)).await.unwrap();
    assert_eq!(tokens.expires_in, 3600);
    assert!(tokens.access_token.starts_with("cmo_"));
    assert_eq!(tokens.scope, "mcp");
    let auth = Auth::new(db);
    assert!(
        auth.authenticate(&tokens.access_token, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        auth.authenticate(&tokens.access_token, "foreign.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        oauth.token(&client, &exchange(&code)).await,
        Err(OAuthError::Grant)
    ));
    assert!(
        auth.authenticate(&tokens.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        oauth
            .token(&client, &refresh(tokens.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
    let code = oauth.issue_code(&authorization).await.unwrap();
    sqlx::query("UPDATE oauth_codes SET expires_at=now()-interval '1 second' WHERE code_hash=$1")
        .bind(Sha256::digest(code.as_bytes()).to_vec())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        oauth.token(&client, &exchange(&code)).await,
        Err(OAuthError::Grant)
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_refresh_rotation_reuse_and_owner_revocation_revoke_the_grant(pool: sqlx::PgPool) {
    let (db, oauth, _, client, authorization) = setup(&pool).await;
    let auth = Auth::new(db);
    let code = oauth.issue_code(&authorization).await.unwrap();
    let first = oauth.token(&client, &exchange(&code)).await.unwrap();
    let original = first.refresh_token.as_deref().unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM oauth_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Sha256::digest(original.as_bytes()).to_vec());
    let mut wrong = refresh(original);
    wrong.scope = Some("admin".into());
    assert!(matches!(
        oauth.token(&client, &wrong).await,
        Err(OAuthError::Scope)
    ));
    let other = oauth.register(metadata(AuthMethod::None)).await.unwrap();
    let other = ClientAuthentication {
        client_id: other.client_id,
        method: AuthMethod::None,
        secret: None,
    };
    assert!(matches!(
        oauth.token(&other, &refresh(original)).await,
        Err(OAuthError::Grant)
    ));
    let second = oauth.token(&client, &refresh(original)).await.unwrap();
    assert_ne!(first.access_token, second.access_token);
    assert!(
        auth.authenticate(&first.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.authenticate(&second.access_token, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        oauth.token(&client, &refresh(original)).await,
        Err(OAuthError::Grant)
    ));
    assert!(
        auth.authenticate(&second.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        oauth
            .token(&client, &refresh(second.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
    let code = oauth.issue_code(&authorization).await.unwrap();
    let tokens = oauth.token(&client, &exchange(&code)).await.unwrap();
    let principal = auth
        .authenticate(&tokens.access_token, "test.example")
        .await
        .unwrap()
        .unwrap();
    assert!(auth.revoke(principal.token_id).await.unwrap());
    assert!(!auth.revoke(principal.token_id).await.unwrap());
    assert!(matches!(
        oauth
            .token(&client, &refresh(tokens.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_confidential_clients_expiry_endpoint_deletion_and_concurrent_exchange(
    pool: sqlx::PgPool,
) {
    let (db, oauth, endpoints, _, mut authorization) = setup(&pool).await;
    let registered = oauth
        .register(metadata(AuthMethod::ClientSecretBasic))
        .await
        .unwrap();
    authorization.client_id = registered.client_id.clone();
    let mut client = ClientAuthentication {
        client_id: registered.client_id,
        method: AuthMethod::None,
        secret: None,
    };
    let code = oauth.issue_code(&authorization).await.unwrap();
    assert!(matches!(
        oauth.token(&client, &exchange(&code)).await,
        Err(OAuthError::Client)
    ));
    client.method = AuthMethod::ClientSecretBasic;
    client.secret = Some("incorrect".into());
    assert!(matches!(
        oauth.token(&client, &exchange(&code)).await,
        Err(OAuthError::Client)
    ));
    client.secret = registered.client_secret;
    let request = exchange(&code);
    let (a, b) = tokio::join!(
        oauth.token(&client, &request),
        oauth.token(&client, &request)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let auth = Auth::new(db);
    let tokens = a.or(b).unwrap();
    assert!(
        auth.authenticate(&tokens.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    let code = oauth.issue_code(&authorization).await.unwrap();
    let tokens = oauth.token(&client, &exchange(&code)).await.unwrap();
    sqlx::query(
        "UPDATE access_tokens SET expires_at=now()-interval '1 second' WHERE token_hash=$1",
    )
    .bind(Sha256::digest(tokens.access_token.as_bytes()).to_vec())
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        auth.authenticate(&tokens.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    let fresh = oauth
        .token(&client, &refresh(tokens.refresh_token.as_deref().unwrap()))
        .await
        .unwrap();
    let endpoint = endpoints
        .get(authorization.workspace_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    endpoints.remove(endpoint.id).await.unwrap();
    authorization.workspace_id = Some(endpoints.create("work", &[]).await.unwrap());
    assert!(
        auth.authenticate(&fresh.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        oauth
            .token(&client, &refresh(fresh.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
    let code = oauth.issue_code(&authorization).await.unwrap();
    let tokens = oauth.token(&client, &exchange(&code)).await.unwrap();
    sqlx::query("UPDATE oauth_grants SET expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        oauth
            .token(&client, &refresh(tokens.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
    oauth.sweep().await.unwrap();
    assert!(
        auth.authenticate(&tokens.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(matches!(
        oauth.prepare_client("https://127.0.0.1/client.json").await,
        Err(OAuthError::Client)
    ));
    let mut invalid = metadata(AuthMethod::None);
    invalid.redirect_uris = vec!["http://evil.example/callback".into()];
    let mut bounded = false;
    for _ in 0..30 {
        match oauth.register(invalid.clone()).await {
            Err(OAuthError::Client) => {}
            Err(OAuthError::Busy) => {
                bounded = true;
                break;
            }
            _ => panic!("invalid registrations must not succeed"),
        }
    }
    assert!(bounded);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_revoke_an_old_oauth_token_revokes_the_refresh_grant_and_active_sibling(
    pool: sqlx::PgPool,
) {
    use codemode_core::components::bulk::Selection;
    let (db, oauth, _, client, authorization) = setup(&pool).await;
    let auth = Auth::new(db);
    let code = oauth.issue_code(&authorization).await.unwrap();
    let first = oauth.token(&client, &exchange(&code)).await.unwrap();
    let first_id = auth
        .authenticate(&first.access_token, "test.example")
        .await
        .unwrap()
        .unwrap()
        .token_id;
    let second = oauth
        .token(&client, &refresh(first.refresh_token.as_deref().unwrap()))
        .await
        .unwrap();
    let second_id = auth
        .authenticate(&second.access_token, "test.example")
        .await
        .unwrap()
        .unwrap()
        .token_id;
    assert_eq!(
        auth.revoke_many(
            Selection {
                ids: Some(vec![first_id]),
                query: None
            },
            1
        )
        .await
        .unwrap(),
        1
    );
    assert!(
        auth.authenticate(&second.access_token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        oauth
            .token(&client, &refresh(second.refresh_token.as_deref().unwrap()))
            .await,
        Err(OAuthError::Grant)
    ));
    let audited: Vec<String> =
        sqlx::query_scalar("SELECT item FROM control_changes WHERE kind='token' ORDER BY item")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(audited, vec![second_id.to_string()]);
}
