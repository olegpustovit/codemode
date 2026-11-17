use super::*;
use codemode_core::components::{
    agent_sessions::{AgentSessions, Error as SessionError},
    sessions::Sessions,
    workspaces::Workspaces,
};

fn service(
    db: Database,
    tasks: TaskTracker,
    stop: CancellationToken,
) -> Arc<Execution<FakeSandbox>> {
    Arc::new(Execution::new(
        db.clone(),
        managed_sources(db.clone(), Catalogue::new(db)),
        Arc::new(FakeSandbox),
        tasks,
        stop,
        Default::default(),
    ))
}
#[sqlx::test(migrations = "./migrations")]
async fn activity_groups_concurrent_starts_isolates_grants_and_workspaces_and_slides_expiry(
    pool: sqlx::PgPool,
) {
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("Fixture agent", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let execution = service(db.clone(), tasks.clone(), stop.clone());
    let history = AgentSessions::new(db.clone());
    let fresh = futures_util::future::join_all(
        (0..6).map(|_| execution.execute(principal.clone(), "# Find issues\nreturn 42".into())),
    )
    .await;
    assert!(fresh.into_iter().all(|r| r.unwrap().status == "completed"));
    let first = history
        .list("", None)
        .await
        .unwrap()
        .sessions
        .pop()
        .unwrap();
    assert_eq!(first.title, "Find issues");
    assert_eq!(first.client, "Fixture agent");
    let results = futures_util::future::join_all(
        (0..6).map(|_| execution.execute(principal.clone(), "return 42".into())),
    )
    .await;
    assert!(
        results
            .into_iter()
            .all(|r| r.unwrap().status == "completed")
    );
    let sessions = history.list("", None).await.unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].executions, 12);
    history
        .rename(first.id, "  Renamed investigation  ")
        .await
        .unwrap();
    execution
        .execute(principal.clone(), "# A later title\nreturn 42".into())
        .await
        .unwrap();
    assert_eq!(
        history.detail(first.id, None).await.unwrap().session.title,
        "Renamed investigation"
    );
    for bad in ["".to_owned(), "x".repeat(161), "line\nbreak".into()] {
        assert!(matches!(
            history.rename(first.id, &bad).await,
            Err(SessionError::Invalid)
        ));
    }
    assert!(matches!(
        history.list(&"x".repeat(1025), None).await,
        Err(SessionError::Invalid)
    ));
    assert_eq!(
        history
            .list("Renamed investigation", None)
            .await
            .unwrap()
            .sessions
            .len(),
        1
    );
    sqlx::query("UPDATE agent_sessions SET last_seen_at=now()-interval '31 minutes' WHERE id=$1")
        .bind(first.id)
        .execute(&pool)
        .await
        .unwrap();
    execution
        .execute(principal.clone(), "return 42".into())
        .await
        .unwrap();
    assert_eq!(history.list("", None).await.unwrap().sessions.len(), 2);
    let workspace = Workspaces::new(db.clone(), "test.example".into())
        .unwrap()
        .create("Other workspace", &[])
        .await
        .unwrap();
    let (_, other) = auth
        .create_for_workspace("Other agent", workspace, "test.example", "/mcp")
        .await
        .unwrap()
        .unwrap();
    let other = auth
        .authenticate(&other, "test.example")
        .await
        .unwrap()
        .unwrap();
    execution.execute(other, "return 42".into()).await.unwrap();
    assert_eq!(history.list("", None).await.unwrap().sessions.len(), 3);
    let (_, second) = auth.create("Second agent", "test.example").await.unwrap();
    let second = auth
        .authenticate(&second, "test.example")
        .await
        .unwrap()
        .unwrap();
    execution.execute(second, "return 42".into()).await.unwrap();
    assert_eq!(history.list("", None).await.unwrap().sessions.len(), 4);
    let expiry:f64=sqlx::query_scalar("SELECT extract(epoch FROM expires_at-last_seen_at)::float8 FROM agent_sessions WHERE id=$1").bind(first.id).fetch_one(&pool).await.unwrap();
    assert_eq!(expiry, 96.0 * 3600.0);
    stop.cancel();
    tasks.close();
    tasks.wait().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_refresh_keeps_the_grant_session_and_expiry_cascades_all_stored_calls(
    pool: sqlx::PgPool,
) {
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (_, token) = auth
        .create("OAuth client label", "test.example")
        .await
        .unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("INSERT INTO oauth_clients(client_id,metadata) VALUES('fixture-client','{}')")
        .execute(&pool)
        .await
        .unwrap();
    let grant:Uuid=sqlx::query_scalar("INSERT INTO oauth_grants(client_id,endpoint_id,scope,refresh_allowed) VALUES('fixture-client',$1,'mcp',true) RETURNING id").bind(principal.workspace_id).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE access_tokens SET oauth_grant=$2 WHERE id=$1")
        .bind(principal.token_id)
        .bind(grant)
        .execute(&pool)
        .await
        .unwrap();
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let execution = service(db.clone(), tasks.clone(), stop.clone());
    let history = AgentSessions::new(db.clone());
    let run = execution
        .execute(principal.clone(), "return 42".into())
        .await
        .unwrap();
    let (_, rotated) = auth
        .create("OAuth client label", "test.example")
        .await
        .unwrap();
    let rotated = auth
        .authenticate(&rotated, "test.example")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE access_tokens SET oauth_grant=$2 WHERE id=$1")
        .bind(rotated.token_id)
        .bind(grant)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE access_tokens SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(principal.token_id)
        .execute(&pool)
        .await
        .unwrap();
    execution
        .execute(rotated, "return 42".into())
        .await
        .unwrap();
    let sessions = history.list("", None).await.unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].executions, 2);
    let id = sessions.sessions[0].id;
    sqlx::query("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome,preview) VALUES($1,0,'fixture.echo','{}','ok','value')").bind(run.execution_id).execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE agent_sessions SET last_seen_at=now()-interval '4 days 1 second' WHERE id=$1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(history.list("", None).await.unwrap().sessions.is_empty());
    assert!(matches!(
        history.detail(id, None).await,
        Err(SessionError::NotFound)
    ));
    assert!(
        codemode_core::components::execution::log::Log::new(db.clone())
            .detail(run.execution_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(history.sweep().await.unwrap(), 1);
    assert_eq!(history.sweep().await.unwrap(), 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM executions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM tool_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    stop.cancel();
    tasks.close();
    tasks.wait().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn sessions_are_tenant_private_and_execution_pages_do_not_repeat_or_expose_foreign_cursors(
    pool: sqlx::PgPool,
) {
    let global = database(&pool).await;
    let accounts = Sessions::new(global.clone());
    let mut users = Vec::new();
    for email in ["ursus95@gmail.com", "other@olegpustovit.com"] {
        let session = accounts
            .register(email, "fixture-account-password".into())
            .await
            .unwrap();
        users.push(
            accounts
                .user(secrecy::ExposeSecret::expose_secret(&session))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let a = global.for_tenant(users[0].tenant_id).await.unwrap();
    let b = global.for_tenant(users[1].tenant_id).await.unwrap();
    let tasks = TaskTracker::new();
    let stop = CancellationToken::new();
    let auth = Auth::new(a.clone());
    let (_, token) = auth.create("Owner agent", "test.example").await.unwrap();
    let owner = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let execution = service(a.clone(), tasks.clone(), stop.clone());
    for _ in 0..22 {
        execution
            .execute(owner.clone(), "return 42".into())
            .await
            .unwrap();
    }
    let history = AgentSessions::new(a.clone());
    let foreign = AgentSessions::new(b.clone());
    let list = history.list("", None).await.unwrap();
    assert_eq!(list.sessions.len(), 1);
    let id = list.sessions[0].id;
    assert!(foreign.list("", None).await.unwrap().sessions.is_empty());
    assert!(foreign.activity(None).await.unwrap().executions.is_empty());
    assert!(matches!(
        foreign.detail(id, None).await,
        Err(SessionError::NotFound)
    ));
    assert!(matches!(
        foreign.rename(id, "stolen").await,
        Err(SessionError::NotFound)
    ));
    assert_eq!(foreign.sweep().await.unwrap(), 0);
    let first = history.detail(id, None).await.unwrap();
    assert_eq!(first.executions.len(), 20);
    let second = history.detail(id, first.next_cursor).await.unwrap();
    assert_eq!(second.executions.len(), 2);
    assert!(second.next_cursor.is_none());
    assert!(!first.executions.iter().any(|one| {
        second
            .executions
            .iter()
            .any(|two| one.summary.id == two.summary.id)
    }));
    assert!(
        history
            .detail(id, Some(Uuid::new_v4()))
            .await
            .unwrap()
            .executions
            .is_empty()
    );
    for _ in 0..51 {
        sqlx::query(
            "UPDATE agent_sessions SET last_seen_at=clock_timestamp()-interval '31 minutes'",
        )
        .execute(&pool)
        .await
        .unwrap();
        execution
            .execute(owner.clone(), "return 42".into())
            .await
            .unwrap();
    }
    let page = history.list("", None).await.unwrap();
    assert_eq!(page.sessions.len(), 50);
    let rest = history.list("", page.next_cursor).await.unwrap();
    assert_eq!(rest.sessions.len(), 2);
    assert!(rest.next_cursor.is_none());
    assert!(
        !page
            .sessions
            .iter()
            .any(|one| rest.sessions.iter().any(|two| one.id == two.id))
    );
    sqlx::query("INSERT INTO executions(token_id,code,status,session_id) SELECT $1,'return 1','failed',$2 FROM generate_series(1,1001)")
        .bind(owner.token_id).bind(id).execute(&pool).await.unwrap();
    let activity = history.activity(None).await.unwrap();
    assert_eq!(activity.executions.len(), 1000);
    assert_eq!(activity.executions[0].status, "failed");
    assert_eq!(activity.executions[0].session_id, id);
    let rest = history.activity(activity.next_cursor).await.unwrap();
    assert_eq!(rest.executions.len(), 74);
    assert!(rest.next_cursor.is_none());
    assert!(
        !activity
            .executions
            .iter()
            .any(|one| rest.executions.iter().any(|two| one.id == two.id))
    );
    assert!(
        foreign
            .activity(activity.next_cursor)
            .await
            .unwrap()
            .executions
            .is_empty()
    );
    sqlx::query("UPDATE agent_sessions SET last_seen_at=now()-interval '5 days'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(history.activity(None).await.unwrap().executions.is_empty());
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    a.close().await;
    b.close().await;
}

#[sqlx::test(migrations = false)]
async fn sessions_migration_backfills_idle_boundaries_titles_grants_and_four_day_deletion(
    pool: sqlx::PgPool,
) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version <= 16)
    {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let workspace:Uuid=sqlx::query_scalar("INSERT INTO endpoints(slug,hostname,all_sources) VALUES('All sources','test.example',true) RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO oauth_clients(client_id,metadata) VALUES('backfill-client','{}')")
        .execute(&pool)
        .await
        .unwrap();
    let grant:Uuid=sqlx::query_scalar("INSERT INTO oauth_grants(client_id,endpoint_id,scope,refresh_allowed) VALUES('backfill-client',$1,'mcp',true) RETURNING id").bind(workspace).fetch_one(&pool).await.unwrap();
    let mut tokens = Vec::new();
    for (name, hash, oauth) in [
        ("Old client", 1, Some(grant)),
        ("Rotated label", 2, Some(grant)),
        ("Other agent", 3, None),
    ] {
        let id:Uuid=sqlx::query_scalar("INSERT INTO access_tokens(name,audience,token_hash,workspace_id,oauth_grant) VALUES($1,'test.example',decode(repeat($2,32),'hex'),$3,$4) RETURNING id").bind(name).bind(format!("{hash:02x}")).bind(workspace).bind(oauth).fetch_one(&pool).await.unwrap();
        tokens.push(id);
    }
    let mut runs = Vec::new();
    for (token, minutes, code) in [
        (tokens[0], 120, "#\n  # Find issues\n# ignored"),
        (tokens[0], 101, "return 42"),
        (tokens[0], 60, "return 42"),
        (tokens[1], 50, "return 43"),
        (tokens[2], 100, "return 44"),
        (tokens[0], 7200, "# expired\nreturn 0"),
    ] {
        let id:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,code,status,started_at,finished_at,tool_calls) VALUES($1,$2,'completed',now()-($3::int*interval '1 minute'),now()-($3::int*interval '1 minute')+interval '10 seconds',1) RETURNING id").bind(token).bind(code).bind(minutes).fetch_one(&pool).await.unwrap();
        runs.push(id);
    }
    for (execution, seq, path) in [
        (runs[1], 0, "sentry.list_issues"),
        (runs[1], 1, "linear.create_issue"),
        (runs[2], 0, "sentry.list_issues"),
        (runs[5], 0, "old.expired"),
    ] {
        sqlx::query("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome) VALUES($1,$2,$3,'{}','ok')").bind(execution).bind(seq).bind(path).execute(&pool).await.unwrap();
    }
    // Legacy tokens without a workspace are retained in their tenant's protected default workspace.
    sqlx::query("UPDATE access_tokens SET workspace_id=NULL WHERE id=$1")
        .bind(tokens[0])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0017_agent_sessions.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let sessions:Vec<(Uuid,String,i64)>=sqlx::query_as("SELECT s.id,s.title,count(e.id) FROM agent_sessions s JOIN executions e ON e.session_id=s.id GROUP BY s.id ORDER BY s.started_at").fetch_all(&pool).await.unwrap();
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].1, "Find issues");
    assert_eq!(sessions[0].2, 2);
    let title: String = sqlx::query_scalar("SELECT title FROM executions WHERE id=$1")
        .bind(runs[1])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title, "sentry.list_issues → linear.create_issue");
    let grouped: Vec<Uuid> = sqlx::query_scalar(
        "SELECT session_id FROM executions WHERE id=ANY($1) ORDER BY started_at",
    )
    .bind(&runs[2..4])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(grouped[0], grouped[1]);
    let archived: i64 = sqlx::query_scalar("SELECT count(*) FROM executions WHERE id=$1")
        .bind(runs[5])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(archived, 0);
    let calls: i64 = sqlx::query_scalar("SELECT count(*) FROM tool_calls WHERE execution_id=$1")
        .bind(runs[5])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls, 0);
    let token_count: i64 = sqlx::query_scalar("SELECT count(*) FROM access_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(token_count, 3);
    let expiry: bool = sqlx::query_scalar(
        "SELECT bool_and(expires_at-last_seen_at=interval '96 hours') FROM agent_sessions",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(expiry);
    assert!(
        sqlx::query("UPDATE executions SET result=repeat('x',16385)")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE tool_calls SET preview=repeat('x',2049)")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn session_memo_rls_rejects_foreign_reads_and_writes(pool: sqlx::PgPool) {
    use codemode_core::components::{
        agent_sessions::{AgentSessions, Error as SessionError, memo::Request},
        sessions::Sessions,
        workspaces::Workspaces,
    };
    use secrecy::ExposeSecret;
    let global = database(&pool).await;
    let browser = Sessions::new(global.clone());
    let mut owned = Vec::new();
    for email in ["ursus95@gmail.com", "alice@olegpustovit.com"] {
        let cookie = browser
            .register(email, "fixture-browser-password".into())
            .await
            .unwrap();
        let user = browser.user(cookie.expose_secret()).await.unwrap().unwrap();
        let db = global.for_tenant(user.tenant_id).await.unwrap();
        let workspaces = Workspaces::new(db.clone(), "test.example".into()).unwrap();
        workspaces.initialize().await.unwrap();
        let workspace = workspaces.default_id().await.unwrap();
        let auth = Auth::new(db.clone());
        let (_, token) = auth
            .create_for_workspace("memo agent", workspace, "test.example", "/mcp")
            .await
            .unwrap()
            .unwrap();
        let principal = auth
            .authenticate(&token, "test.example")
            .await
            .unwrap()
            .unwrap();
        let service = AgentSessions::new(db.clone());
        let session = service.touch(&principal).await.unwrap();
        let execution: Uuid = sqlx::query_scalar("INSERT INTO executions(token_id,session_id,code,status) VALUES($1,$2,'return None','completed') RETURNING id").bind(principal.token_id).bind(session).fetch_one(&pool).await.unwrap();
        service
            .memo(
                execution,
                workspace,
                Request::Set {
                    key: "team".into(),
                    value: serde_json::json!(email),
                },
            )
            .await
            .unwrap();
        owned.push((db, user.tenant_id, session, workspace, execution));
    }
    for (own, other) in [(0, 1), (1, 0)] {
        let service = AgentSessions::new(owned[own].0.clone());
        assert!(matches!(
            service
                .memo(
                    owned[other].4,
                    owned[other].3,
                    Request::Get { key: "team".into() }
                )
                .await,
            Err(SessionError::NotFound)
        ));
        assert!(matches!(
            service
                .memo(
                    owned[other].4,
                    owned[other].3,
                    Request::Set {
                        key: "team".into(),
                        value: serde_json::json!("foreign")
                    }
                )
                .await,
            Err(SessionError::NotFound)
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
            sqlx::query_scalar("SELECT count(*) FROM agent_session_memo WHERE session_id=$1")
                .bind(owned[other].2)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(foreign, 0);
        let altered = sqlx::query("UPDATE agent_session_memo SET data='{}' WHERE session_id=$1")
            .bind(owned[other].2)
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(altered.rows_affected(), 0);
        assert!(sqlx::query("INSERT INTO agent_session_memo(session_id,tenant_id,data) VALUES($1,$2,'{}') ON CONFLICT(session_id) DO UPDATE SET tenant_id=EXCLUDED.tenant_id,data=EXCLUDED.data").bind(owned[other].2).bind(owned[own].1).execute(&mut *tx).await.is_err());
        tx.rollback().await.unwrap();
    }
    for (db, ..) in owned {
        db.close().await;
    }
}
