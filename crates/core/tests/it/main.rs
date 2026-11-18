use codemode_core::{
    components::{
        auth::Auth,
        catalogue::{Catalogue, Definition, SearchError},
        execution::{Error, Execution},
        sources::Sources,
    },
    infrastructure::{
        postgres::Database,
        sandbox::{CallResult, Sandbox, Step},
    },
};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use sqlx::ConnectOptions;
use std::sync::Arc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;
mod access;
mod agent_sessions;
mod bulk;
mod endpoints;
mod graphql;
mod identity;
mod linked;
mod oauth;
mod openapi;
mod policies;
mod scripts;
mod sessions;
mod sources;
mod tenancy;
mod upstream_oauth;

#[sqlx::test(migrations = "./migrations")]
async fn binary_rollback_accepts_newer_migrations_but_checks_known_checksums(pool: sqlx::PgPool) {
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES(999,'future additive migration',true,'\\x00'::bytea,0)").execute(&pool).await.unwrap();
    let url = pool.connect_options().to_url_lossy();
    let db = Database::connect(url.as_str()).await.unwrap();
    assert!(db.healthy().await);
    assert_eq!(db.schema_version(), 999);
    db.close().await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum='\\x00'::bytea WHERE version=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(Database::connect(url.as_str()).await.is_err());
}

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

async fn database(pool: &sqlx::PgPool) -> Database {
    let db = Database::connect(pool.connect_options().to_url_lossy().as_str())
        .await
        .unwrap();
    assert_eq!(db.schema_version(), 25);
    db
}

fn managed_sources(db: Database, catalogue: Catalogue) -> Sources {
    let accounts = codemode_core::components::accounts::Accounts::new(
        db.clone(),
        Some([73; 32]),
        "test".into(),
    )
    .unwrap();
    let endpoints =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    Sources::new(db, catalogue, accounts, endpoints)
}

#[sqlx::test(migrations = "./migrations")]
async fn token_is_hashed_audience_bound_and_revocable(pool: sqlx::PgPool) {
    let auth = Auth::new(database(&pool).await);
    let (id, token) = auth.create("test", "work.microapps.space").await.unwrap();
    let principal = auth
        .authenticate(&token, "work.microapps.space")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(principal.token_id, id);
    assert!(
        auth.authenticate(&token, "home.microapps.space")
            .await
            .unwrap()
            .is_none()
    );
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM access_tokens WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Sha256::digest(token.as_bytes()).to_vec());
    assert_eq!(auth.list().await.unwrap().len(), 1);
    assert!(auth.revoke(id).await.unwrap());
    assert!(!auth.revoke(id).await.unwrap());
    assert!(
        auth.authenticate(&token, "work.microapps.space")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.authenticate("invalid", "work.microapps.space")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn replacement_search_and_cursor_contract(pool: sqlx::PgPool) {
    let source: Uuid = sqlx::query_scalar(
        "INSERT INTO sources(slug,kind,config) VALUES('example','mcp_http','{}') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let catalogue = Catalogue::new(database(&pool).await);
    let definitions: Vec<_> = (0..22).map(|i| Definition {
            upstream_name: format!("listIssues{i:02}"),
            path: format!("example.list_issues_{i:02}"),
            description: "List issues.\nDetailed documentation.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{"repo":{"type":"string"}},"required":["repo"]}),
            output_schema: None,
            read_only: true,
            destructive: false,
        }).collect();
    catalogue.replace(source, &definitions, 0).await.unwrap();
    for name in ["listIssues00", "list_issues_00", "example.list_issues_00"] {
        let found = catalogue.search(name, false, None, None).await.unwrap();
        assert_eq!(found.tools.len(), 1, "{name}");
        assert_eq!(found.tools[0].path, "example.list_issues_00");
    }
    sqlx::query("UPDATE tools SET search=setweight(to_tsvector('simple','example list issues 00'),'A') WHERE path='example.list_issues_00'").execute(&pool).await.unwrap();
    assert!(
        catalogue
            .search("listIssues00", false, None, None)
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    sqlx::raw_sql(include_str!(
        "../../migrations/0011_compound_tool_names.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        catalogue
            .search("listIssues00", false, None, None)
            .await
            .unwrap()
            .tools[0]
            .path,
        "example.list_issues_00"
    );
    let first = catalogue.search("issues", false, None, None).await.unwrap();
    let wire = serde_json::to_value(&first).unwrap();
    assert!(wire.get("nextCursor").is_some());
    assert!(wire.get("cursor").is_none());
    assert_eq!(first.tools.len(), 10);
    assert_eq!(first.tools[0].description, "List issues.");
    let cursor = first.cursor.unwrap();
    let second = catalogue
        .search("issues", false, Some(&cursor), None)
        .await
        .unwrap();
    assert_eq!(second.tools.len(), 10);
    let third = catalogue
        .search("issues", false, second.cursor.as_deref(), None)
        .await
        .unwrap();
    assert_eq!(third.tools.len(), 2);
    assert!(third.cursor.is_none());
    assert!(second.tools.last().unwrap().path < third.tools[0].path);
    assert!(first.tools.last().unwrap().path < second.tools[0].path);
    assert!(matches!(
        catalogue.search("other", false, Some(&cursor), None).await,
        Err(SearchError::Invalid)
    ));
    assert!(matches!(
        catalogue.search("issues", true, Some(&cursor), None).await,
        Err(SearchError::Invalid)
    ));
    assert_eq!(
        catalogue
            .search("", true, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        10
    );
    let full = catalogue.search("", true, None, None).await.unwrap();
    assert_eq!(
        full.tools
            .iter()
            .filter(|t| t.signature.contains(" -> "))
            .count(),
        3
    );
    assert!(full.tools.iter().all(|t| !t.blocked));
    catalogue
        .replace(source, &definitions[..1], 0)
        .await
        .unwrap();
    assert!(
        catalogue
            .replace(source, &[definitions[0].clone(), definitions[0].clone()], 0)
            .await
            .is_err()
    );
    assert_eq!(
        catalogue
            .search("", false, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
    sqlx::query("UPDATE sources SET revision=revision+1 WHERE id=$1")
        .bind(source)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!catalogue.replace(source, &definitions, 0).await.unwrap());
    assert_eq!(
        catalogue
            .search("", true, None, None)
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
}

struct FakeSandbox;

impl Sandbox for FakeSandbox {
    type Run = ();
    async fn start(
        &self,
        code: &str,
        _: &str,
    ) -> Result<((), Step), codemode_core::infrastructure::sandbox::Error> {
        if code == "fail" {
            return Err(codemode_core::infrastructure::sandbox::Error::Invalid(
                "test failure".into(),
            ));
        }
        if code == "too_many" {
            return Ok((
                (),
                Step::Calls(
                    (0..101)
                        .map(|id| codemode_core::infrastructure::sandbox::PendingCall {
                            id,
                            path: "missing.tool".into(),
                            args: RawValue::from_string("{}".into()).unwrap(),
                        })
                        .collect(),
                ),
            ));
        }
        Ok((
            (),
            Step::Done {
                result: RawValue::from_string("42".into()).unwrap(),
                output: "printed\n".into(),
                output_dropped_bytes: 0,
            },
        ))
    }
    async fn resume(
        &self,
        _: (),
        _: &[(u32, CallResult)],
    ) -> Result<((), Step), codemode_core::infrastructure::sandbox::Error> {
        unreachable!()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn completion_and_failure_are_audited(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("test", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue);
    let tasks = TaskTracker::new();
    let log = codemode_core::components::execution::log::Log::new(db.clone());
    let service = Arc::new(Execution::new(
        db,
        sources,
        Arc::new(FakeSandbox),
        tasks.clone(),
        CancellationToken::new(),
        Default::default(),
    ));
    for (code, status) in [
        ("return 42", "completed"),
        ("fail", "failed"),
        ("too_many", "failed"),
    ] {
        let result = service
            .execute(principal.clone(), code.into())
            .await
            .unwrap();
        assert_eq!(result.status, status);
        let stored: (String, String, bool) = sqlx::query_as(
            "SELECT code,status,finished_at IS NOT NULL FROM executions WHERE id=$1",
        )
        .bind(result.execution_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, (code.into(), status.into(), true));
        let detail = log.detail(result.execution_id).await.unwrap().unwrap();
        assert_eq!(detail.code, code);
        assert_eq!(detail.summary.status, status);
        assert!(detail.summary.started_at.ends_with('Z'));
        assert!(detail.calls.is_empty());
        if status == "completed" {
            assert_eq!(result.result.get(), "42");
            assert_eq!(result.output, "printed\n");
        } else {
            assert!(result.error.is_some());
        }
        if code == "too_many" {
            assert_eq!(result.tool_calls, 0);
        }
    }
    assert!(matches!(
        service.execute(principal, "x".repeat(65537)).await,
        Err(Error::Source)
    ));
    tasks.close();
    tasks.wait().await;
    let first = log.list(None).await.unwrap();
    assert_eq!(first.len(), 3);
    let remaining = log.list(Some(first[0].id)).await.unwrap();
    assert_eq!(
        remaining.iter().map(|r| r.id).collect::<Vec<_>>(),
        first[1..].iter().map(|r| r.id).collect::<Vec<_>>()
    );
    assert!(log.detail(Uuid::new_v4()).await.unwrap().is_none());
}

mod remote;

mod stdio;

mod pool;

#[sqlx::test(migrations = "./migrations")]
async fn discovery_finds_status_prefixes_and_scopes_typo_suggestions(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let workspaces =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    workspaces.initialize().await.unwrap();
    let source: Uuid = sqlx::query_scalar("INSERT INTO sources(slug,python_name,kind,config) VALUES('nudge','nudge','mcp_http','{}') RETURNING id").fetch_one(&pool).await.unwrap();
    let workspace = workspaces.create("Nudge", &[source]).await.unwrap();
    let catalogue = Catalogue::new(db);
    let definitions = [
        ("list_issue_statuses", "List workflow states for a team."),
        ("create_issue", "Create an issue with a status ID."),
    ]
    .map(|(name, description)| Definition {
        upstream_name: name.into(),
        path: format!("nudge.{name}"),
        description: description.into(),
        input_schema: serde_json::json!({"type":"object"}),
        output_schema: None,
        read_only: true,
        destructive: false,
    });
    catalogue.replace(source, &definitions, 0).await.unwrap();
    for query in ["nudge status", "nudge list_statuses", "nudge listStatus"] {
        let page = catalogue
            .search_scoped(query, false, None, None, Some(workspace))
            .await
            .unwrap();
        assert_eq!(page.tools[0].path, "nudge.list_issue_statuses", "{query}");
        assert!(page.suggestions.is_empty(), "{query}");
    }
    let empty = workspaces.create("Empty", &[]).await.unwrap();
    for (id, expected) in [
        (workspace, Some("nudge.list_issue_statuses")),
        (empty, None),
    ] {
        let page = catalogue
            .search_scoped("nudge.list_isssue_statuses", false, None, None, Some(id))
            .await
            .unwrap();
        assert!(page.tools.is_empty());
        assert_eq!(page.suggestions.first().map(|m| m.path.as_str()), expected);
        assert!(page.suggestions.iter().all(|m| m.suggested && !m.blocked));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn discovery_manifest_prioritizes_useful_entry_points_over_alphabetical_noise(
    pool: sqlx::PgPool,
) {
    let db = database(&pool).await;
    let workspaces =
        codemode_core::components::workspaces::Workspaces::new(db.clone(), "test.example".into())
            .unwrap();
    workspaces.initialize().await.unwrap();
    let workspace = workspaces.default_id().await.unwrap();
    let source: Uuid = sqlx::query_scalar("INSERT INTO sources(slug,python_name,kind,config) VALUES('nudge','nudge','mcp_http','{}') RETURNING id").fetch_one(&pool).await.unwrap();
    let names = [
        "aaa0",
        "aaa1",
        "aaa2",
        "aaa3",
        "aaa4",
        "aaa5",
        "list_issues",
    ];
    let definitions: Vec<_> = names
        .into_iter()
        .map(|name| Definition {
            upstream_name: name.into(),
            path: format!("nudge.{name}"),
            description: name.into(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            read_only: true,
            destructive: false,
        })
        .collect();
    let catalogue = Catalogue::new(db);
    let empty = catalogue.manifest(workspace).await.unwrap();
    assert!(empty.contains("nudge (0:"));
    catalogue.replace(source, &definitions, 0).await.unwrap();
    let manifest = catalogue.manifest(workspace).await.unwrap();
    assert!(manifest.contains("list_issues"));
    assert!(manifest.contains("7 tools"));
    assert!(!manifest.contains("aaa5"));
    assert!(manifest.len() <= 3072);
    let alternatives = catalogue
        .search_scoped("list_issues OR aaa0", false, None, None, Some(workspace))
        .await
        .unwrap();
    assert_eq!(alternatives.tools.len(), 2);
    let conjunction = catalogue
        .search_scoped("list_issues aaa0", false, None, None, Some(workspace))
        .await
        .unwrap();
    assert!(conjunction.tools.is_empty());
    workspaces.set_all_tools(workspace, false).await.unwrap();
    let excluded = catalogue.manifest(workspace).await.unwrap();
    assert!(excluded.contains("nudge (0:"));
    assert!(!excluded.contains("list_issues"));
}
