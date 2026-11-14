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
