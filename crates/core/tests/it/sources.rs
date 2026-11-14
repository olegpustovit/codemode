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
        .save(None, http("work_name", "https://one.example/mcp"))
        .await
        .unwrap();
    let original = sources.get(first).await.unwrap().unwrap();
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
        .save(Some(first), http("renamed", "https://one.example/mcp"))
        .await
        .unwrap();
    assert_eq!(accounts.list().await.unwrap().len(), 1);
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
