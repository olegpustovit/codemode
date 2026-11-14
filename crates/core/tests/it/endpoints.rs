use super::*;
use codemode_core::components::workspaces::{Error as WorkspaceError, Workspaces};

#[sqlx::test(migrations = "./migrations")]
async fn workspace_membership_is_transactional_and_tokens_revoked_on_delete(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let workspaces = Workspaces::new(db.clone(), "test.example".into()).unwrap();
    workspaces.initialize().await.unwrap();
    workspaces.initialize().await.unwrap();
    let main = workspaces.default_id().await.unwrap();
    assert!(workspaces.get(main).await.unwrap().unwrap().all_sources);
    assert!(matches!(
        workspaces.remove(main).await,
        Err(WorkspaceError::Reserved)
    ));
    assert!(matches!(
        workspaces.update(main, "All sources", &[]).await,
        Err(WorkspaceError::Reserved)
    ));
    for name in ["", "  ", "line\nbreak", &"x".repeat(81)] {
        assert!(matches!(
            workspaces.create(name, &[]).await,
            Err(WorkspaceError::Invalid)
        ));
    }
    let source: Uuid = sqlx::query_scalar(
        "INSERT INTO sources(slug,kind,config) VALUES('fixture','mcp_http','{}') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(matches!(
        workspaces.create("missing", &[Uuid::new_v4()]).await,
        Err(WorkspaceError::Sources)
    ));
    assert_eq!(workspaces.list().await.unwrap().len(), 1);
    assert!(matches!(
        workspaces.create("duplicate", &[source, source]).await,
        Err(WorkspaceError::Sources)
    ));
    let id = workspaces.create("Work projects", &[source]).await.unwrap();
    assert!(matches!(
        workspaces.create("Work projects", &[]).await,
        Err(WorkspaceError::Conflict)
    ));
    assert_eq!(workspaces.sources(id).await.unwrap(), Some(vec![source]));
    assert!(matches!(
        workspaces.sources(Uuid::new_v4()).await,
        Err(WorkspaceError::NotFound)
    ));
    assert!(workspaces.sources(main).await.unwrap().is_none());
    assert!(matches!(
        workspaces.update(id, "Renamed", &[Uuid::new_v4()]).await,
        Err(WorkspaceError::Sources)
    ));
    assert_eq!(
        workspaces.get(id).await.unwrap().unwrap().name,
        "Work projects"
    );
    assert_eq!(workspaces.sources(id).await.unwrap(), Some(vec![source]));
    workspaces.update(id, "Renamed", &[]).await.unwrap();
    assert_eq!(workspaces.sources(id).await.unwrap(), Some(vec![]));
    let auth = Auth::new(db);
    let (_, token) = auth
        .create_for_workspace("work", id, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(principal.workspace_id, id);
    assert_eq!(principal.resource, "/mcp");
    assert!(workspaces.remove(id).await.unwrap());
    assert!(
        auth.create_for_workspace("deleted", id, "test.example", "/mcp")
            .await
            .unwrap()
            .is_none()
    );
    let new_id = workspaces.create("Renamed", &[source]).await.unwrap();
    assert_ne!(id, new_id);
    assert!(
        auth.authenticate(&token, "test.example")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = false)]
async fn workspace_upgrade_preserves_connections_ids_membership_scripts_and_history(
    pool: sqlx::PgPool,
) {
    let mut legacy = sqlx::migrate!("./migrations");
    legacy.migrations =
        std::borrow::Cow::Owned(legacy.iter().filter(|m| m.version <= 14).cloned().collect());
    legacy.run(&pool).await.unwrap();
    let main:Uuid=sqlx::query_scalar("INSERT INTO endpoints(slug,hostname,all_sources) VALUES('test','test.example',true) RETURNING id").fetch_one(&pool).await.unwrap();
    let work: Uuid = sqlx::query_scalar(
        "INSERT INTO endpoints(slug,hostname) VALUES('work','work.example') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let source: Uuid = sqlx::query_scalar(
        "INSERT INTO sources(slug,kind,config) VALUES('fixture','mcp_http','{}') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO endpoint_sources(endpoint_id,source_id) VALUES($1,$2)")
        .bind(work)
        .bind(source)
        .execute(&pool)
        .await
        .unwrap();
    let cipher = vec![91u8; 32];
    sqlx::query("INSERT INTO accounts(source_id,kind,ciphertext,nonce,key_id) VALUES($1,'bearer',$2,$3,'fixture')").bind(source).bind(&cipher).bind(vec![42u8;24]).execute(&pool).await.unwrap();
    let script:Uuid=sqlx::query_scalar("INSERT INTO saved_scripts(endpoint_id,name,description,code) VALUES($1,'Keep me','','return 42') RETURNING id").bind(work).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO oauth_clients(client_id,metadata) VALUES('fixture','{}')")
        .execute(&pool)
        .await
        .unwrap();
    let mut grants = Vec::new();
    let mut tokens = Vec::new();
    for (id, host, byte) in [(main, "test.example", 1u8), (work, "work.example", 2u8)] {
        let grant:Uuid=sqlx::query_scalar("INSERT INTO oauth_grants(client_id,endpoint_id,scope,refresh_allowed) VALUES('fixture',$1,'mcp',true) RETURNING id").bind(id).fetch_one(&pool).await.unwrap();
        let token:Uuid=sqlx::query_scalar("INSERT INTO access_tokens(name,audience,token_hash,oauth_grant) VALUES('fixture',$1,$2,$3) RETURNING id").bind(host).bind(vec![byte;32]).bind(grant).fetch_one(&pool).await.unwrap();
        grants.push(grant);
        tokens.push(token);
    }
    let execution:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,code,status) VALUES($1,'return 42','completed') RETURNING id").bind(tokens[1]).fetch_one(&pool).await.unwrap();
    let db = database(&pool).await;
    let workspaces = Workspaces::new(db, "test.example".into()).unwrap();
    assert_eq!(workspaces.default_id().await.unwrap(), main);
    assert_eq!(workspaces.get(work).await.unwrap().unwrap().name, "work");
    assert_eq!(workspaces.sources(work).await.unwrap(), Some(vec![source]));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT ciphertext FROM accounts WHERE source_id=$1")
        .bind(source)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, cipher);
    let stored: (Uuid, String) =
        sqlx::query_as("SELECT endpoint_id,code FROM saved_scripts WHERE id=$1")
            .bind(script)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, (work, "return 42".into()));
    let stored: Uuid = sqlx::query_scalar("SELECT token_id FROM executions WHERE id=$1")
        .bind(execution)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, tokens[1]);
    for (index, workspace, revoked) in [(0, main, false), (1, work, true)] {
        let stored: (Uuid, String, bool) = sqlx::query_as(
            "SELECT workspace_id,resource,revoked_at IS NOT NULL FROM access_tokens WHERE id=$1",
        )
        .bind(tokens[index])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, (workspace, "/mcp".into(), revoked));
        let stored: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM oauth_grants WHERE id=$1")
                .bind(grants[index])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, revoked);
    }
}
