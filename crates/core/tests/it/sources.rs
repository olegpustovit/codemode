use super::*;
use codemode_core::components::{
    accounts::Accounts,
    sources::{Config, Error as SourceError, Source, Transport},
};

fn http(slug: &str, url: &str) -> Source {
    Source {
        slug: slug.into(),
        requires_account: false,
        transport: Transport::McpHttp {
            url: url.into(),
            bearer_env: None,
        },
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn managed_sources_keep_names_and_invalidate_credentials_when_destination_changes(
    pool: sqlx::PgPool,
) {
    let db = database(&pool).await;
    let sources = managed_sources(db.clone(), Catalogue::new(db.clone()));
    let accounts = Accounts::new(db, Some([73; 32]), "test".into()).unwrap();
    let first = sources
        .save(None, http("WORK_name", "https://one.example/mcp"))
        .await
        .unwrap();
    let original = sources.get(first).await.unwrap().unwrap();
    assert_eq!(original.source.slug, "work_name");
    assert_eq!(original.python_name, "work_name");
    assert!(matches!(
        sources
            .save(None, http("work_name", "https://other.example/mcp"))
            .await,
        Err(SourceError::Conflict)
    ));
    let second = sources
        .save(None, http("work-name", "https://two.example/mcp"))
        .await
        .unwrap();
    assert_eq!(
        sources.get(first).await.unwrap().unwrap().python_name,
        "work_name"
    );
    assert_eq!(
        sources.get(second).await.unwrap().unwrap().python_name,
        "work_name_2"
    );
    accounts
        .link_bearer(first, "provider_credential_alpha".into())
        .await
        .unwrap();
    sources
        .save(Some(first), http("Renamed", "https://one.example/mcp"))
        .await
        .unwrap();
    assert_eq!(accounts.list().await.unwrap().len(), 1);
    assert_eq!(
        sources.get(first).await.unwrap().unwrap().source.slug,
        "Renamed"
    );
    assert_eq!(
        sources.get(first).await.unwrap().unwrap().python_name,
        "work_name"
    );
    sources
        .save(Some(first), http("renamed", "https://new.example/mcp"))
        .await
        .unwrap();
    assert!(accounts.list().await.unwrap().is_empty());
    assert!(sources.get(first).await.unwrap().unwrap().revision > original.revision);
    assert!(sources.remove(first).await.unwrap());
    assert!(sources.get(first).await.unwrap().is_none());
    assert!(!sources.remove(first).await.unwrap());
    assert!(matches!(
        sources.resync(first).await,
        Err(SourceError::NotFound)
    ));
    let mut invalid = http("bad", "https://example.com/mcp");
    if let Transport::McpHttp { bearer_env, .. } = &mut invalid.transport {
        *bearer_env = Some("HOME".into());
    }
    assert!(matches!(
        sources.save(None, invalid).await,
        Err(SourceError::Config(_))
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn source_file_is_imported_once_with_secrets_removed_from_config(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let sources = managed_sources(db.clone(), Catalogue::new(db));
    let path = std::env::temp_dir().join(format!("codemode-sources-{}.toml", Uuid::new_v4()));
    tokio::fs::write(&path,"[[sources]]\nslug='legacy'\nkind='mcp_http'\nurl='https://example.com/mcp'\nbearer_env='HOME'\n").await.unwrap();
    sources.import_file(&path).await.unwrap();
    tokio::fs::remove_file(&path).await.unwrap();
    sources.import_file(&path).await.unwrap();
    sources
        .import(Config {
            sources: vec![http("ignored", "https://ignored.example/mcp")],
        })
        .await
        .unwrap();
    let records = sources.list().await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].source.requires_account);
    assert!(matches!(
        &records[0].source.transport,
        Transport::McpHttp {
            bearer_env: None,
            ..
        }
    ));
    let imported: bool = sqlx::query_scalar("SELECT singleton FROM source_import")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(imported);
    let credential_kind: String = sqlx::query_scalar("SELECT kind FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(credential_kind, "bearer");
}

#[sqlx::test(migrations = "./migrations")]
async fn integrations_reuse_connections_and_merge_only_the_current_tenants_link_state(
    pool: sqlx::PgPool,
) {
    use codemode_core::components::integrations;
    let db = database(&pool).await;
    let sources = managed_sources(db.clone(), Catalogue::new(db.clone()));
    let github = integrations::add(&sources, "github").await.unwrap();
    assert_eq!(integrations::add(&sources, "github").await.unwrap(), github);
    assert_eq!(sources.list().await.unwrap().len(), 1);
    let row = integrations::list(&sources)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.entry.slug == "github")
        .unwrap();
    assert_eq!(row.source_id, Some(github));
    assert_eq!(row.state, "needs_link");
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    accounts
        .link_bearer(github, "fixture-integration-token".into())
        .await
        .unwrap();
    assert_eq!(integrations::add(&sources, "github").await.unwrap(), github);
    assert_eq!(accounts.list().await.unwrap().len(), 1);
    let row = integrations::list(&sources)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.entry.slug == "github")
        .unwrap();
    assert_eq!(row.state, "pending");
    let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants DEFAULT VALUES RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let private = db.for_tenant(tenant).await.unwrap();
    let other = managed_sources(private.clone(), Catalogue::new(private.clone()));
    let row = integrations::list(&other)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.entry.slug == "github")
        .unwrap();
    assert_eq!(row.source_id, None);
    assert_eq!(row.state, "available");
    assert!(matches!(
        integrations::add(&other, "missing").await,
        Err(integrations::Error::NotFound)
    ));
    assert!(other.list().await.unwrap().is_empty());
    private.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_apps_are_sealed_tenant_private_and_independent_of_linked_accounts(
    pool: sqlx::PgPool,
) {
    use codemode_core::components::{
        accounts::{Error as AccountError, OAuthAppInput},
        integrations,
    };
    let global = database(&pool).await;
    let db = global.for_tenant(Uuid::nil()).await.unwrap();
    let sources = managed_sources(db.clone(), Catalogue::new(db.clone()));
    let accounts = Accounts::new(db.clone(), Some([73; 32]), "test".into()).unwrap();
    let source = integrations::add(&sources, "github").await.unwrap();
    let app = |id: &str, secret: &str| OAuthAppInput {
        client_id: id.to_owned().into(),
        client_secret: secret.to_owned().into(),
    };
    accounts
        .save_oauth_app("github", app("fixture-client-id", "fixture-client-secret"))
        .await
        .unwrap();
    assert_eq!(accounts.configured_apps().await.unwrap(), ["github"]);
    assert!(accounts.list().await.unwrap().is_empty());
    let row: (Uuid, Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT id,ciphertext,nonce FROM oauth_apps WHERE provider='github'")
            .fetch_one(&pool)
            .await
            .unwrap();
    for secret in ["fixture-client-id", "fixture-client-secret"] {
        assert!(!row.1.windows(secret.len()).any(|v| v == secret.as_bytes()));
    }
    assert_eq!(row.2.len(), 24);
    let (first, second) = tokio::join!(
        accounts.save_oauth_app("github", app("client-a", "secret-a")),
        accounts.save_oauth_app("github", app("client-b", "secret-b"))
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM oauth_apps WHERE provider='github'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        row.0
    );
    let url = accounts
        .start_oauth(source, "test.example", "fixture-session")
        .await
        .unwrap();
    let url = url::Url::parse(&url).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert!(["client-a", "client-b"].contains(&params["client_id"].as_str()));
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(
        params["redirect_uri"],
        "https://test.example/api/accounts/oauth/callback"
    );
    assert!(!url.as_str().contains("secret-a") && !url.as_str().contains("secret-b"));
    let state = &params["state"];
    accounts
        .save_oauth_app("github", app("replacement-client", "replacement-secret"))
        .await
        .unwrap();
    assert!(matches!(
        accounts
            .complete_oauth(state, "fixture-session", Some("unused-code"), None, None)
            .await,
        Err(AccountError::Changed)
    ));
    use base64::Engine as _;
    let url = url::Url::parse(
        &accounts
            .start_oauth(source, "test.example", "fixture-session")
            .await
            .unwrap(),
    )
    .unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let state = &params["state"];
    let basic =
        base64::engine::general_purpose::STANDARD.encode("replacement-client:replacement-secret");
    let reason = format!(
        "access_denied: app policy rejected access {state} replacement-secret {basic} unused-code"
    );
    let result = accounts
        .complete_oauth(
            state,
            "fixture-session",
            Some("unused-code"),
            None,
            Some(&reason),
        )
        .await
        .unwrap();
    let codemode_core::components::accounts::OAuthCompletion::Denied(id, detail) = result else {
        panic!("expected provider denial")
    };
    assert_eq!(id, source);
    assert!(detail.contains("step 5") && detail.contains("app policy rejected access"));
    for secret in [
        state.as_str(),
        "replacement-secret",
        basic.as_str(),
        "unused-code",
    ] {
        assert!(!detail.contains(secret));
    }
    assert!(accounts.list().await.unwrap().is_empty());
    let wrong_key = Accounts::new(db.clone(), Some([74; 32]), "test".into()).unwrap();
    assert!(matches!(
        wrong_key
            .start_oauth(source, "test.example", "fixture-session")
            .await,
        Err(AccountError::Authentication)
    ));
    let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants DEFAULT VALUES RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let private = db.for_tenant(tenant).await.unwrap();
    let other = accounts.with_database(private.clone());
    assert!(other.configured_apps().await.unwrap().is_empty());
    other
        .save_oauth_app("github", app("other-client", "other-secret"))
        .await
        .unwrap();
    assert_eq!(other.configured_apps().await.unwrap(), ["github"]);
    accounts
        .link_bearer(source, "fixture-linked-token".into())
        .await
        .unwrap();
    accounts.unlink(source).await.unwrap();
    assert_eq!(accounts.configured_apps().await.unwrap(), ["github"]);
    accounts
        .link_bearer(source, "fixture-linked-token".into())
        .await
        .unwrap();
    accounts.remove_oauth_app("github").await.unwrap();
    assert!(accounts.configured_apps().await.unwrap().is_empty());
    assert_eq!(accounts.list().await.unwrap().len(), 1);
    assert_eq!(other.configured_apps().await.unwrap(), ["github"]);
    assert!(matches!(
        other
            .save_oauth_app("linear", app("unused", "unused"))
            .await,
        Err(AccountError::Provider)
    ));
    private.close().await;
    db.close().await;
}
