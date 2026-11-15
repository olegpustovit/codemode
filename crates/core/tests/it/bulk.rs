use super::*;
use codemode_core::components::{
    bulk::{Error as BulkError, Selection},
    policies::{Action, BulkAction, Policies},
    scripts::{Content, Revision, Scripts},
    sources::{Source, Transport},
    workspaces::Workspaces,
};

async fn tenant(
    pool: &sqlx::PgPool,
    root: &Database,
    tools: usize,
) -> (Database, Workspaces, Uuid, Sources, Uuid) {
    let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants DEFAULT VALUES RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap();
    let db = root.for_tenant(tenant).await.unwrap();
    let workspaces = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    workspaces.initialize().await.unwrap();
    let workspace = workspaces.default_id().await.unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue.clone());
    let source = sources
        .save(
            None,
            Source {
                slug: "github".into(),
                requires_account: false,
                transport: Transport::McpHttp {
                    url: "https://fixture.example/mcp".into(),
                    bearer_env: None,
                },
            },
        )
        .await
        .unwrap();
    let definitions: Vec<_> = (0..tools)
        .map(|index| Definition {
            path: format!(
                "github.{}_{index:03}",
                if index % 2 == 0 { "delete" } else { "get" }
            ),
            upstream_name: format!("tool_{index:03}"),
            description: "Fixture tool".into(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            read_only: index % 2 != 0,
            destructive: index % 2 == 0,
        })
        .collect();
    catalogue
        .replace(
            source,
            &definitions,
            sources.get(source).await.unwrap().unwrap().revision,
        )
        .await
        .unwrap();
    (db, workspaces, workspace, sources, source)
}
fn query(pattern: &str) -> Selection<String> {
    Selection {
        ids: None,
        query: Some(pattern.into()),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_policy_patterns_page_all_300_tools_and_isolate_tenants_workspaces_and_audits(
    pool: sqlx::PgPool,
) {
    let root = database(&pool).await;
    let (a, workspaces, workspace, _, source) = tenant(&pool, &root, 300).await;
    let (b, _, other, _, _) = tenant(&pool, &root, 4).await;
    let policies = Policies::new(a.clone());
    let mut paths = std::collections::BTreeSet::new();
    let mut cursor = None;
    loop {
        let page = policies
            .list(workspace, "github.*", cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(page.total_matches, 300);
        assert!(page.tools.len() <= 50);
        paths.extend(page.tools.into_iter().map(|tool| tool.path));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(paths.len(), 300);
    assert_eq!(
        policies
            .bulk(workspace, query("*.delete_*"), 150, BulkAction::Block)
            .await
            .unwrap(),
        150
    );
    assert_eq!(policies.blocked(workspace).await.unwrap().len(), 150);
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM control_changes WHERE workspace_id=$1 AND kind='policy' AND action='block'").bind(workspace).fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 150);
    assert!(
        Policies::new(b.clone())
            .blocked(other)
            .await
            .unwrap()
            .is_empty()
    );
    let sibling = workspaces
        .create("Other workspace", &[source])
        .await
        .unwrap();
    assert!(policies.blocked(sibling).await.unwrap().is_empty());
    assert!(matches!(
        Policies::new(b.clone())
            .bulk(workspace, query("*"), 300, BulkAction::Block)
            .await,
        Err(BulkError::Changed)
    ));
    let mixed = Selection {
        ids: Some(vec!["github.delete_000".into(), "foreign.tool".into()]),
        query: None,
    };
    assert!(matches!(
        policies.bulk(workspace, mixed, 2, BulkAction::Allow).await,
        Err(BulkError::Changed)
    ));
    assert_eq!(
        policies
            .effective(workspace, source, "tool_000")
            .await
            .unwrap(),
        Action::Block
    );
    assert!(matches!(
        policies
            .bulk(workspace, query("github.*"), 299, BulkAction::Allow)
            .await,
        Err(BulkError::Changed)
    ));
    assert_eq!(policies.blocked(workspace).await.unwrap().len(), 150);
    assert!(matches!(
        policies
            .bulk(workspace, query("github.*"), 300, BulkAction::Exclude)
            .await,
        Err(BulkError::Scope)
    ));
    workspaces.set_all_tools(workspace, false).await.unwrap();
    assert_eq!(
        policies
            .bulk(workspace, query("github.*"), 300, BulkAction::Include)
            .await
            .unwrap(),
        300
    );
    assert_eq!(
        policies
            .bulk(workspace, query("*.delete_*"), 150, BulkAction::Exclude)
            .await
            .unwrap(),
        150
    );
    let included = policies.list(workspace, "*.delete_*", None).await.unwrap();
    assert!(included.tools.iter().all(|tool| !tool.in_scope));
    let other_audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM control_changes WHERE workspace_id=$1")
            .bind(other)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(other_audits, 0);
    a.close().await;
    b.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_scripts_reject_stale_revisions_and_mixed_tenants_before_deleting_anything(
    pool: sqlx::PgPool,
) {
    let root = database(&pool).await;
    let (a, _, workspace, _, _) = tenant(&pool, &root, 0).await;
    let (b, _, other, _, _) = tenant(&pool, &root, 0).await;
    let scripts = Scripts::new(a.clone());
    let content = |name: &str| Content {
        name: name.into(),
        description: "Daily report".into(),
        code: "1".into(),
    };
    let first = scripts
        .save(workspace, None, content("Report A"))
        .await
        .unwrap();
    let second = scripts
        .save(workspace, None, content("Report B"))
        .await
        .unwrap();
    let foreign = Scripts::new(b.clone())
        .save(other, None, content("Report A"))
        .await
        .unwrap();
    let ids = || Selection {
        ids: Some(vec![first.summary.id, second.summary.id]),
        query: None,
    };
    let revisions = vec![
        Revision {
            id: first.summary.id,
            revision: first.summary.revision,
        },
        Revision {
            id: second.summary.id,
            revision: second.summary.revision,
        },
    ];
    scripts
        .save(
            workspace,
            Some((second.summary.id, second.summary.revision)),
            content("Report B updated"),
        )
        .await
        .unwrap();
    assert!(matches!(
        scripts.remove_many(workspace, ids(), 2, &revisions).await,
        Err(BulkError::Changed)
    ));
    assert_eq!(scripts.list(workspace).await.unwrap().len(), 2);
    let mixed = Selection {
        ids: Some(vec![first.summary.id, foreign.summary.id]),
        query: None,
    };
    let mixed_revisions = vec![
        Revision {
            id: first.summary.id,
            revision: 1,
        },
        Revision {
            id: foreign.summary.id,
            revision: 1,
        },
    ];
    assert!(matches!(
        scripts
            .remove_many(workspace, mixed, 2, &mixed_revisions)
            .await,
        Err(BulkError::Changed)
    ));
    let latest = scripts.list(workspace).await.unwrap();
    let revisions: Vec<_> = latest
        .iter()
        .map(|script| Revision {
            id: script.id,
            revision: script.revision,
        })
        .collect();
    assert_eq!(
        scripts
            .remove_many(
                workspace,
                Selection {
                    ids: None,
                    query: Some("Report*".into())
                },
                2,
                &revisions
            )
            .await
            .unwrap(),
        2
    );
    assert!(scripts.list(workspace).await.unwrap().is_empty());
    assert_eq!(Scripts::new(b.clone()).list(other).await.unwrap().len(), 1);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM control_changes WHERE workspace_id=$1 AND kind='script'",
    )
    .bind(workspace)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);
    a.close().await;
    b.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_source_sync_and_personal_token_revocation_keep_other_tenants_private(
    pool: sqlx::PgPool,
) {
    let root = database(&pool).await;
    let (a, _, workspace, sources, source) = tenant(&pool, &root, 0).await;
    let (b, _, other, other_sources, other_source) = tenant(&pool, &root, 0).await;
    let revision = sources.get(source).await.unwrap().unwrap().revision;
    let mixed = Selection {
        ids: Some(vec![source, other_source]),
        query: None,
    };
    assert!(matches!(
        sources.sync_many(mixed, 2).await,
        Err(BulkError::Changed)
    ));
    assert_eq!(
        sources.get(source).await.unwrap().unwrap().revision,
        revision
    );
    assert_eq!(
        sources
            .sync_many(
                Selection {
                    ids: None,
                    query: Some("git*".into())
                },
                1
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sources.get(source).await.unwrap().unwrap().revision,
        revision + 1
    );
    assert_eq!(
        other_sources
            .get(other_source)
            .await
            .unwrap()
            .unwrap()
            .revision,
        revision
    );
    let auth = Auth::new(a.clone());
    let first = auth
        .create_for_workspace("Bulk A", workspace, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let second = auth
        .create_for_workspace("Bulk B", workspace, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let foreign_auth = Auth::new(b.clone());
    let foreign = foreign_auth
        .create_for_workspace("Bulk A", other, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        auth.revoke_many(
            Selection {
                ids: Some(vec![first.0, foreign.0]),
                query: None
            },
            2
        )
        .await,
        Err(BulkError::Changed)
    ));
    assert!(
        auth.authenticate(&first.1, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        auth.revoke_many(
            Selection {
                ids: None,
                query: Some("Bulk*".into())
            },
            2
        )
        .await
        .unwrap(),
        2
    );
    for token in [first.1, second.1] {
        assert!(
            auth.authenticate(&token, "test.example")
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        foreign_auth
            .authenticate(&foreign.1, "test.example")
            .await
            .unwrap()
            .is_some()
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM control_changes WHERE workspace_id=$1 AND kind='token'",
    )
    .bind(workspace)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);
    a.close().await;
    b.close().await;
}
