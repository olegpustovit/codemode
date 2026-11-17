use super::*;
use codemode_core::components::{
    scripts::{Content, Error as ScriptError, Scripts},
    workspaces::Workspaces,
};
fn content(name: &str, code: &str) -> Content {
    Content {
        name: name.into(),
        description: "Summarize project issues".into(),
        code: code.into(),
    }
}
#[sqlx::test(migrations = "./migrations")]
async fn saved_scripts_crud_revisions_limits_and_endpoint_isolation(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let endpoints = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    endpoints.initialize().await.unwrap();
    let main = endpoints.default_id().await.unwrap();
    let other = endpoints.create("other", &[]).await.unwrap();
    let scripts = Scripts::new(db.clone());
    let first = scripts
        .save(main, None, content("  Weekly issues  ", "return 42"))
        .await
        .unwrap();
    let id = first.summary.id;
    assert_eq!(first.summary.name, "Weekly issues");
    assert_eq!(first.summary.revision, 1);
    assert_eq!(scripts.list(main).await.unwrap().len(), 1);
    let summary = serde_json::to_value(scripts.list(main).await.unwrap()).unwrap();
    assert!(summary[0].get("code").is_none());
    assert_eq!(scripts.search(main, "issues").await.unwrap()[0].id, id);
    assert!(scripts.search(other, "issues").await.unwrap().is_empty());
    assert!(matches!(
        scripts.get(other, id).await,
        Err(ScriptError::NotFound)
    ));
    assert!(matches!(
        scripts.code_for(other, id).await,
        Err(ScriptError::NotFound)
    ));
    assert!(matches!(
        scripts
            .save(main, None, content("Weekly issues", "return 1"))
            .await,
        Err(ScriptError::Conflict)
    ));
    let updated = scripts
        .save(main, Some((id, 1)), content("Weekly issues", "return 43"))
        .await
        .unwrap();
    assert_eq!(updated.summary.revision, 2);
    assert_eq!(updated.code, "return 43");
    assert!(matches!(
        scripts
            .save(main, Some((id, 1)), content("stale", "return 0"))
            .await,
        Err(ScriptError::Changed)
    ));
    assert!(matches!(
        scripts.remove(main, id, 1).await,
        Err(ScriptError::Changed)
    ));
    assert!(matches!(
        scripts.remove(other, id, 2).await,
        Err(ScriptError::NotFound)
    ));
    assert_eq!(scripts.get(main, id).await.unwrap().code, "return 43");
    for invalid in [
        content("   ", ""),
        content("line\nbreak", ""),
        content(&"a".repeat(121), ""),
        content("large", &"é".repeat(32769)),
        content("nul", "\0"),
    ] {
        assert!(matches!(
            scripts.save(main, None, invalid).await,
            Err(ScriptError::Invalid)
        ));
    }
    let mut long_description = content("description", "");
    long_description.description = "é".repeat(2001);
    assert!(matches!(
        scripts.save(main, None, long_description).await,
        Err(ScriptError::Invalid)
    ));
    scripts
        .save(main, None, content("byte boundary", &"é".repeat(32768)))
        .await
        .unwrap();
    sqlx::query("INSERT INTO saved_scripts(endpoint_id,name,description,code) SELECT $1,'seed '||i,'','' FROM generate_series(1,126) i").bind(main).execute(&pool).await.unwrap();
    assert_eq!(scripts.list(main).await.unwrap().len(), 128);
    assert_eq!(scripts.search(main, "").await.unwrap().len(), 20);
    assert!(matches!(
        scripts.save(main, None, content("one too many", "")).await,
        Err(ScriptError::Limit)
    ));
    scripts
        .save(main, Some((id, 2)), content("renamed", "return 44"))
        .await
        .unwrap();
    scripts.remove(main, id, 3).await.unwrap();
    assert!(matches!(
        scripts.get(main, id).await,
        Err(ScriptError::NotFound)
    ));
    scripts
        .save(other, None, content("scoped", "return 7"))
        .await
        .unwrap();
    assert_eq!(scripts.list(other).await.unwrap().len(), 1);
    endpoints.remove(other).await.unwrap();
    assert!(scripts.list(other).await.unwrap().is_empty());
    db.close().await;
}

fn moneybag_definition(alias: &str) -> Definition {
    Definition {
        upstream_name: "entries_list".into(),
        path: format!("{alias}.entries_list"),
        description: "List entries".into(),
        input_schema: serde_json::json!({"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"},"account":{"type":"string"},"cursor":{"type":"integer"}}}),
        output_schema: Some(
            serde_json::json!({"type":"object","properties":{"items":{"type":"array","items":{}},"next_cursor":{"type":"integer"}}}),
        ),
        read_only: true,
        destructive: false,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn source_recipes_install_once_preserve_edits_deletion_scope_and_script_quota(
    pool: sqlx::PgPool,
) {
    let db = database(&pool).await;
    let workspaces = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    workspaces.initialize().await.unwrap();
    let main = workspaces.default_id().await.unwrap();
    let source: Uuid = sqlx::query_scalar("INSERT INTO sources(slug,python_name,kind,config) VALUES('ledger','ledger','mcp_http',$1) RETURNING id")
        .bind(serde_json::json!({"slug":"ledger","kind":"mcp_http","url":"https://moneybag.microapps.space/mcp"})).fetch_one(&pool).await.unwrap();
    Catalogue::new(db.clone())
        .replace(source, &[moneybag_definition("ledger")], 0)
        .await
        .unwrap();
    let scripts = Scripts::new(db.clone());
    let found = scripts.search(main, "monthly").await.unwrap();
    assert_eq!(found.len(), 1);
    let id = found[0].id;
    assert_eq!(scripts.search(main, "monthly").await.unwrap()[0].id, id);
    let original = scripts.get(main, id).await.unwrap();
    assert!(original.code.contains("tools.ledger.entries_list.all"));
    let edited = scripts
        .save(
            main,
            Some((id, original.summary.revision)),
            content(&original.summary.name, "return args"),
        )
        .await
        .unwrap();
    assert_eq!(scripts.list(main).await.unwrap().len(), 1);
    assert_eq!(scripts.get(main, id).await.unwrap().code, "return args");
    scripts
        .remove(main, id, edited.summary.revision)
        .await
        .unwrap();
    assert!(scripts.list(main).await.unwrap().is_empty());
    let empty = workspaces.create("Empty", &[]).await.unwrap();
    assert!(scripts.list(empty).await.unwrap().is_empty());
    let scoped = workspaces.create("Scoped", &[source]).await.unwrap();
    workspaces.set_all_tools(scoped, false).await.unwrap();
    assert!(scripts.list(scoped).await.unwrap().is_empty());
    let full = workspaces.create("Full", &[source]).await.unwrap();
    sqlx::query("INSERT INTO saved_scripts(endpoint_id,name,description,code) SELECT $1,'custom-'||n,'','return 1' FROM generate_series(1,128) n").bind(full).execute(&pool).await.unwrap();
    assert_eq!(scripts.list(full).await.unwrap().len(), 128);
    let installed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM source_recipe_installs WHERE workspace_id=$1")
            .bind(full)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(installed, 0);
    sqlx::query("DELETE FROM saved_scripts WHERE endpoint_id=$1 AND name='custom-1'")
        .bind(full)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(scripts.list(full).await.unwrap().len(), 128);
    assert_eq!(scripts.search(full, "monthly").await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn source_recipe_installation_is_tenant_private(pool: sqlx::PgPool) {
    use codemode_core::components::sessions::Sessions;
    use secrecy::ExposeSecret;
    let global = database(&pool).await;
    let sessions = Sessions::new(global.clone());
    let mut owned = Vec::new();
    for email in ["ursus95@gmail.com", "alice@olegpustovit.com"] {
        let cookie = sessions
            .register(email, "fixture-browser-password".into())
            .await
            .unwrap();
        let user = sessions
            .user(cookie.expose_secret())
            .await
            .unwrap()
            .unwrap();
        let db = global.for_tenant(user.tenant_id).await.unwrap();
        let workspace = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        workspace.initialize().await.unwrap();
        let workspace = workspace.default_id().await.unwrap();
        let source: Uuid = sqlx::query_scalar("INSERT INTO sources(tenant_id,slug,python_name,kind,config) VALUES($1,'ledger','ledger','mcp_http',$2) RETURNING id")
            .bind(user.tenant_id).bind(serde_json::json!({"slug":"ledger","kind":"mcp_http","url":"https://moneybag.microapps.space/mcp"})).fetch_one(&pool).await.unwrap();
        Catalogue::new(db.clone())
            .replace(source, &[moneybag_definition("ledger")], 0)
            .await
            .unwrap();
        let scripts = Scripts::new(db.clone());
        let found = scripts.search(workspace, "monthly").await.unwrap();
        assert_eq!(found.len(), 1);
        owned.push((db, user.tenant_id, workspace, source, found[0].id));
    }
    assert_ne!(owned[0].4, owned[1].4);
    for (own, other) in [(0, 1), (1, 0)] {
        let scripts = Scripts::new(owned[own].0.clone());
        assert!(
            scripts
                .search(owned[other].2, "monthly")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            scripts.code_for(owned[own].2, owned[other].4).await,
            Err(ScriptError::NotFound)
        ));
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL ROLE codemode_tenant")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT set_config('codemode.tenant_id',$1,true)")
            .bind(owned[own].1.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let foreign: i64 =
            sqlx::query_scalar("SELECT count(*) FROM source_recipe_installs WHERE workspace_id=$1")
                .bind(owned[other].2)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(foreign, 0);
        assert!(sqlx::query("INSERT INTO source_recipe_installs(workspace_id,source_id,recipe) VALUES($1,$2,'foreign')").bind(owned[other].2).bind(owned[other].3).execute(&mut *tx).await.is_err());
        tx.rollback().await.unwrap();
    }
    for (db, ..) in owned {
        db.close().await;
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn nudge_recipe_is_discoverable_for_its_scoped_source(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let workspaces = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    workspaces.initialize().await.unwrap();
    let main = workspaces.default_id().await.unwrap();
    let source: Uuid = sqlx::query_scalar("INSERT INTO sources(slug,python_name,kind,config) VALUES('issues','issues','mcp_http',$1) RETURNING id").bind(serde_json::json!({"slug":"issues","kind":"mcp_http","url":"https://nudge.microapps.space/mcp"})).fetch_one(&pool).await.unwrap();
    let paged = serde_json::json!({"properties":{"data":{"properties":{"items":{"items":{}},"nextCursor":{"type":"string"}}}}});
    let definitions: Vec<_> = ["list_teams", "list_issue_statuses", "create_issue"]
        .into_iter()
        .map(|name| Definition {
            upstream_name: name.into(),
            path: format!("issues.{name}"),
            description: name.into(),
            input_schema: if name == "create_issue" {
                serde_json::json!({"properties":{"teamId":{},"statusId":{},"title":{}}})
            } else {
                serde_json::json!({"properties":{"cursor":{"type":"string"}}})
            },
            output_schema: Some(paged.clone()),
            read_only: name != "create_issue",
            destructive: false,
        })
        .collect();
    Catalogue::new(db.clone())
        .replace(source, &definitions, 0)
        .await
        .unwrap();
    let scripts = Scripts::new(db.clone());
    let found = scripts.search(main, "create issue").await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].description.contains("teamKey"));
    assert!(found[0].description.contains("statusName"));
    let code = scripts.code_for(main, found[0].id).await.unwrap();
    assert!(code.contains("tools.issues.create_issue"));
    assert!(!code.contains("$SOURCE"));
    let empty = workspaces.create("No sources", &[]).await.unwrap();
    assert!(
        scripts
            .search(empty, "create issue")
            .await
            .unwrap()
            .is_empty()
    );
}
