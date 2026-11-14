use super::*;
use codemode_core::components::{
    accounts::Accounts,
    auth::oauth::{AuthMethod, AuthorizationRequest, ClientAuthentication, OAuth, TokenRequest},
    scripts::{Content, Scripts},
    sessions::Sessions,
    workspaces::Workspaces,
};
use secrecy::ExposeSecret;

#[sqlx::test(migrations = "./migrations")]
async fn tenant_pools_isolate_identical_mcp_names_credentials_tokens_scripts_and_audits(
    pool: sqlx::PgPool,
) {
    let global = database(&pool).await;
    let sessions = Sessions::new(global.clone());
    let mut users = Vec::new();
    for email in ["ursus95@gmail.com", "alice@olegpustovit.com"] {
        let token = sessions
            .register(email, "fixture-browser-password".into())
            .await
            .unwrap();
        users.push(sessions.user(token.expose_secret()).await.unwrap().unwrap());
    }
    let a = global.for_tenant(users[0].tenant_id).await.unwrap();
    let b = global.for_tenant(users[1].tenant_id).await.unwrap();
    let mut sources = Vec::new();
    let mut endpoints = Vec::new();
    let mut tokens = Vec::new();
    let mut scripts = Vec::new();
    for (db, label) in [(&a, "tenant-a"), (&b, "tenant-b")] {
        let source=managed_sources(db.clone(),Catalogue::new(db.clone())).save(None,serde_json::from_value(serde_json::json!({"slug":"same","kind":"mcp_http","url":"https://mcp.example/mcp"})).unwrap()).await.unwrap();
        Catalogue::new(db.clone())
            .replace(
                source,
                &[Definition {
                    upstream_name: "read".into(),
                    path: "same.read".into(),
                    description: label.into(),
                    input_schema: serde_json::json!({"type":"object"}),
                    output_schema: None,
                    read_only: true,
                    destructive: false,
                }],
                0,
            )
            .await
            .unwrap();
        let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
        accounts
            .link_bearer(source, format!("secret-{label}").into())
            .await
            .unwrap();
        let ep = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        ep.initialize().await.unwrap();
        let endpoint = ep.default_id().await.unwrap();
        let (_, token) = Auth::new(db.clone())
            .create_for_workspace(label, endpoint, "test.example", "/mcp")
            .await
            .unwrap()
            .unwrap();
        let principal = Auth::new(db.clone())
            .authenticate(&token, "test.example")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            principal.tenant_id,
            if label == "tenant-a" {
                users[0].tenant_id
            } else {
                users[1].tenant_id
            }
        );
        sqlx::query("INSERT INTO executions(token_id,code,status) VALUES($1,$2,'completed')")
            .bind(principal.token_id)
            .bind(label)
            .execute(&pool)
            .await
            .unwrap();
        let script = Scripts::new(db.clone())
            .save(
                endpoint,
                None,
                Content {
                    name: "same".into(),
                    description: label.into(),
                    code: "return 1".into(),
                },
            )
            .await
            .unwrap();
        sources.push(source);
        endpoints.push(endpoint);
        tokens.push(token);
        scripts.push(script.summary.id);
    }
    for (db, own, other) in [(&a, 0, 1), (&b, 1, 0)] {
        let managed = managed_sources(db.clone(), Catalogue::new(db.clone()));
        assert_eq!(managed.list().await.unwrap().len(), 1);
        assert!(managed.get(sources[other]).await.unwrap().is_none());
        assert!(!managed.remove(sources[other]).await.unwrap());
        let results = Catalogue::new(db.clone())
            .search("", false, None, None)
            .await
            .unwrap();
        assert_eq!(results.tools.len(), 1);
        assert_eq!(
            results.tools[0].description,
            if own == 0 { "tenant-a" } else { "tenant-b" }
        );
        let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
        assert_eq!(accounts.list().await.unwrap()[0].source_id, sources[own]);
        assert!(
            accounts
                .link_bearer(sources[other], "foreign-secret".into())
                .await
                .is_err()
        );
        assert_eq!(Auth::new(db.clone()).list().await.unwrap().len(), 1);
        assert!(
            Auth::new(db.clone())
                .authenticate(&tokens[other], "test.example")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !Auth::new(db.clone())
                .revoke(
                    Auth::new(global.clone())
                        .authenticate(&tokens[other], "test.example")
                        .await
                        .unwrap()
                        .unwrap()
                        .token_id
                )
                .await
                .unwrap()
        );
        assert!(
            Scripts::new(db.clone())
                .code_for(endpoints[own], scripts[other])
                .await
                .is_err()
        );
        assert_eq!(
            codemode_core::components::execution::log::Log::new(db.clone())
                .list(None)
                .await
                .unwrap()
                .len(),
            1
        );
        let ep = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        assert!(
            ep.create(if own == 0 { "first" } else { "second" }, &[sources[other]])
                .await
                .is_err()
        );
        assert!(ep.get(endpoints[other]).await.unwrap().is_none());
        assert_eq!(ep.default_id().await.unwrap(), endpoints[own]);
    }
    let registered = OAuth::new(global.clone(), "test.example".into()).register(serde_json::from_value(serde_json::json!({
        "client_name":"Tenant client", "redirect_uris":["https://client.example/callback"],
        "token_endpoint_auth_method":"none", "grant_types":["authorization_code","refresh_token"]
    })).unwrap()).await.unwrap();
    let auth = ClientAuthentication {
        client_id: registered.client_id.clone(),
        method: AuthMethod::None,
        secret: None,
    };
    for (db, own) in [(&a, 0), (&b, 1)] {
        let oauth = OAuth::new(db.clone(), "test.example".into());
        let request = AuthorizationRequest {
            workspace_id: Some(endpoints[own]),
            response_type: "code".into(),
            client_id: registered.client_id.clone(),
            redirect_uri: "https://client.example/callback".into(),
            resource: "https://test.example/mcp".into(),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into(),
            code_challenge_method: "S256".into(),
            state: None,
            scope: Some("mcp".into()),
        };
        assert_eq!(
            oauth.consent(&request).await.unwrap().workspace_id,
            Some(endpoints[own])
        );
        let code = oauth.issue_code(&request).await.unwrap();
        let response = OAuth::new(global.clone(), "test.example".into())
            .token(
                &auth,
                &TokenRequest {
                    grant_type: "authorization_code".into(),
                    resource: request.resource.clone(),
                    code: Some(code.into()),
                    redirect_uri: Some(request.redirect_uri.clone()),
                    code_verifier: Some("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into()),
                    refresh_token: None,
                    scope: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            Auth::new(global.clone())
                .authenticate(&response.access_token, "test.example")
                .await
                .unwrap()
                .unwrap()
                .tenant_id,
            users[own].tenant_id
        );
        let other = if own == 0 { &b } else { &a };
        assert!(
            Auth::new(other.clone())
                .authenticate(&response.access_token, "test.example")
                .await
                .unwrap()
                .is_none()
        );
        let ep = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        ep.create(if own == 0 { "one" } else { "two" }, &[])
            .await
            .unwrap();
    }
    for (db, foreign) in [(&a, endpoints[1]), (&b, endpoints[0])] {
        let request = AuthorizationRequest {
            response_type: "code".into(),
            client_id: registered.client_id.clone(),
            redirect_uri: "https://client.example/callback".into(),
            resource: "https://test.example/mcp".into(),
            workspace_id: Some(foreign),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into(),
            code_challenge_method: "S256".into(),
            state: None,
            scope: Some("mcp".into()),
        };
        assert!(matches!(
            OAuth::new(db.clone(), "test.example".into())
                .consent(&request)
                .await,
            Err(codemode_core::components::auth::oauth::Error::Target)
        ));
        assert!(
            OAuth::new(db.clone(), "test.example".into())
                .issue_code(&request)
                .await
                .is_err()
        );
    }
    // Closing/reopening pools must retain their immutable tenant identity.
    for user in users {
        let db = global.for_tenant(user.tenant_id).await.unwrap();
        assert_eq!(
            managed_sources(db.clone(), Catalogue::new(db))
                .list()
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
