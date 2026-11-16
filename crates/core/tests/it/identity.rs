use super::*;
use codemode_core::components::{
    accounts::Accounts,
    scripts::{Content, Scripts},
    sessions::{IdentityError, Sessions},
    workspaces::Workspaces,
};
use secrecy::ExposeSecret;

async fn snapshot(pool: &sqlx::PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_array((SELECT jsonb_agg(to_jsonb(u) ORDER BY id) FROM users u),(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tenants t),(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s),(SELECT jsonb_agg(to_jsonb(a) ORDER BY source_id) FROM accounts a),(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM endpoints e),(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM access_tokens a),(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM saved_scripts s),(SELECT jsonb_agg(to_jsonb(g) ORDER BY id) FROM oauth_grants g),(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM executions e),(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM tool_calls c),(SELECT jsonb_agg(to_jsonb(p) ORDER BY endpoint_id,source_id,upstream_name) FROM tool_policies p))")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn identity_links_preserve_tenants_credentials_roles_agents_scripts_and_history(
    pool: sqlx::PgPool,
) {
    let global = database(&pool).await;
    let sessions = Sessions::new(global.clone());
    let mut users = Vec::new();
    let mut legacy = Vec::new();
    let mut tenants = Vec::new();
    let mut sources = Vec::new();
    let mut tokens = Vec::new();
    for (index, email) in ["ursus95@gmail.com", "eval-member@olegpustovit.com"]
        .into_iter()
        .enumerate()
    {
        let token = sessions
            .register(email, "fixture-browser-password".into())
            .await
            .unwrap();
        let user = sessions.user(token.expose_secret()).await.unwrap().unwrap();
        let tenant = global.for_tenant(user.tenant_id).await.unwrap();
        let workspaces = Workspaces::new(tenant.clone(), "test.example".into()).unwrap();
        workspaces.initialize().await.unwrap();
        let workspace = workspaces.default_id().await.unwrap();
        let source = managed_sources(tenant.clone(), Catalogue::new(tenant.clone()))
            .save(None,serde_json::from_value(serde_json::json!({"slug":"private","kind":"mcp_http","url":"https://mcp.example/mcp"})).unwrap()).await.unwrap();
        Accounts::new(tenant.clone(), Some([73; 32]), "test".into())
            .unwrap()
            .link_bearer(source, format!("keep-provider-secret-{index}").into())
            .await
            .unwrap();
        Scripts::new(tenant.clone())
            .save(
                workspace,
                None,
                Content {
                    name: "Keep script".into(),
                    description: "Keep description".into(),
                    code: "return 1".into(),
                },
            )
            .await
            .unwrap();
        let (token_id, agent) = Auth::new(tenant.clone())
            .create_for_workspace("Keep agent", workspace, "test.example", "/mcp")
            .await
            .unwrap()
            .unwrap();
        let execution:Uuid=sqlx::query_scalar("INSERT INTO executions(token_id,code,status,finished_at) VALUES($1,'return 1','completed',now()) RETURNING id").bind(token_id).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome,duration_ms) VALUES($1,1,'private.read','{}','ok',1)").bind(execution).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO oauth_clients(client_id,metadata) VALUES($1,'{}')")
            .bind(format!("fixture-client-{index}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO oauth_grants(client_id,endpoint_id,scope,refresh_allowed) VALUES($1,$2,'mcp:tools',true)").bind(format!("fixture-client-{index}")).bind(workspace).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tool_policies(endpoint_id,source_id,upstream_name,action) VALUES($1,$2,'read','block')").bind(workspace).bind(source).execute(&pool).await.unwrap();
        legacy.push(token);
        users.push(user);
        tenants.push(tenant);
        sources.push(source);
        tokens.push(agent);
    }
    let before = snapshot(&pool).await;
    let linked = futures_util::future::join_all(
        (0..12).map(|_| sessions.link_identity("owner-subject", " URSUS95@GMAIL.COM ")),
    )
    .await;
    for user in linked {
        let user = user.unwrap();
        assert_eq!(user.id, users[0].id);
        assert_eq!(user.tenant_id, users[0].tenant_id);
        assert!(user.admin);
    }
    let member = sessions
        .link_identity("member-subject", "eval-member@olegpustovit.com")
        .await
        .unwrap();
    assert_eq!(member.id, users[1].id);
    assert!(!member.admin);
    assert_eq!(member.tenant_id, users[1].tenant_id);
    // A verified email change never reassigns an already-bound subject or its tenant.
    let canonical = sessions
        .link_identity("owner-subject", "eval-member@olegpustovit.com")
        .await
        .unwrap();
    assert_eq!(canonical.id, users[0].id);
    assert_eq!(canonical.email, users[0].email);
    assert!(matches!(
        sessions
            .link_identity("another-subject", "ursus95@gmail.com")
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        sessions
            .link_identity("unknown-subject", "eval-unknown@olegpustovit.com")
            .await,
        Err(IdentityError::AccountRequired)
    ));
    assert_eq!(snapshot(&pool).await, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    for (index, tenant) in tenants.iter().enumerate() {
        let other = 1 - index;
        assert!(
            managed_sources(tenant.clone(), Catalogue::new(tenant.clone()))
                .get(sources[other])
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            Auth::new(tenant.clone())
                .authenticate(&tokens[other], "test.example")
                .await
                .unwrap()
                .is_none()
        );
        let mut private = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL ROLE codemode_tenant")
            .execute(&mut *private)
            .await
            .unwrap();
        let denied = sqlx::query("SELECT subject FROM user_identities")
            .fetch_all(&mut *private)
            .await
            .unwrap_err();
        assert!(
            matches!(denied,sqlx::Error::Database(ref error) if error.code().as_deref()==Some("42501"))
        );
        private.rollback().await.unwrap();
        assert_eq!(
            Auth::new(tenant.clone())
                .authenticate(&tokens[index], "test.example")
                .await
                .unwrap()
                .unwrap()
                .tenant_id,
            users[index].tenant_id
        );
        let public = serde_json::to_value(&users[index]).unwrap();
        assert_eq!(public.as_object().unwrap().len(), 4);
        assert!(public.get("subject").is_none());
        assert_eq!(
            sessions
                .user(legacy[index].expose_secret())
                .await
                .unwrap()
                .unwrap()
                .id,
            users[index].id
        );
    }
    let logged_in = sessions
        .login("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions
            .user(logged_in.expose_secret())
            .await
            .unwrap()
            .unwrap()
            .id,
        users[0].id
    );
    for tenant in tenants {
        tenant.close().await;
    }
    global.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn identity_rejects_invalid_claims_and_cannot_bootstrap_or_expose_tenants(
    pool: sqlx::PgPool,
) {
    let global = database(&pool).await;
    let sessions = Sessions::new(global.clone());
    for (subject, email) in [
        ("", "owner@example.test"),
        ("with\0nul", "owner@example.test"),
        ("subject", "a..b@example.test"),
        ("subject", "owner@example.test\nextra"),
        ("subject", "a@bad@domain.test"),
        ("subject", "owner@-invalid.test"),
    ] {
        assert!(matches!(
            sessions.link_identity(subject, email).await,
            Err(IdentityError::InvalidClaims)
        ));
    }
    assert!(matches!(
        sessions
            .link_identity(&"x".repeat(256), "owner@example.test")
            .await,
        Err(IdentityError::InvalidClaims)
    ));
    assert!(matches!(
        sessions.link_identity("unknown", "ursus95@gmail.com").await,
        Err(IdentityError::AccountRequired)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenants")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // Existing legacy users can link even when their address is outside signup policy.
    assert!(
        sessions
            .register("legacy@example.test", "fixture-browser-password".into())
            .await
            .is_err()
    );
    let hash =
        codemode_core::components::sessions::hash_password("fixture-browser-password").unwrap();
    let user:Uuid=sqlx::query_scalar("INSERT INTO users(email,password_hash,tenant_id,admin) VALUES('legacy@example.test',$1,$2,false) RETURNING id").bind(&hash).bind(Uuid::nil()).fetch_one(&pool).await.unwrap();
    sqlx::query("CREATE FUNCTION reject_identity_fixture() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'owned identity fixture'; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_identity_fixture BEFORE INSERT ON user_identities FOR EACH ROW EXECUTE FUNCTION reject_identity_fixture()").execute(&pool).await.unwrap();
    assert!(matches!(
        sessions
            .link_identity("legacy-subject", "LEGACY@example.test")
            .await,
        Err(IdentityError::Database(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id=$1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap(),
        hash
    );
    sqlx::query("DROP TRIGGER reject_identity_fixture ON user_identities")
        .execute(&pool)
        .await
        .unwrap();
    let linked = sessions
        .link_identity("legacy-subject", "LEGACY@example.test")
        .await
        .unwrap();
    assert_eq!(linked.id, user);
    assert_eq!(linked.tenant_id, Uuid::nil());
    assert!(!linked.admin);
    global.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn identity_initial_races_and_database_constraints_keep_one_binding_per_user(
    pool: sqlx::PgPool,
) {
    let global = database(&pool).await;
    let sessions = Sessions::new(global.clone());
    let mut users = Vec::new();
    for email in ["ursus95@gmail.com", "eval-second@olegpustovit.com"] {
        let token = sessions
            .register(email, "fixture-browser-password".into())
            .await
            .unwrap();
        users.push(sessions.user(token.expose_secret()).await.unwrap().unwrap());
    }
    let (a, b) = tokio::join!(
        sessions.link_identity("same-subject", &users[0].email),
        sessions.link_identity("same-subject", &users[1].email)
    );
    let canonical = a.unwrap();
    assert_eq!(canonical.id, b.unwrap().id);
    let other = users.iter().find(|user| user.id != canonical.id).unwrap();
    let (a, b) = tokio::join!(
        sessions.link_identity("subject-a", &other.email),
        sessions.link_identity("subject-b", &other.email)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        if a.is_err() { a } else { b },
        Err(IdentityError::Conflict)
    ));
    for subject in [String::new(), "x".repeat(256), "界".repeat(86)] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM user_identities WHERE user_id=$1")
            .bind(other.id)
            .execute(&mut *tx)
            .await
            .unwrap();
        let error = sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES($1,$2)")
            .bind(subject)
            .bind(other.id)
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert!(
            matches!(error,sqlx::Error::Database(ref error) if error.code().as_deref()==Some("23514"))
        );
        tx.rollback().await.unwrap();
    }
    let missing =
        sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES('foreign-user',$1)")
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await
            .unwrap_err();
    assert!(
        matches!(missing,sqlx::Error::Database(ref error) if error.code().as_deref()==Some("23503"))
    );
    let duplicate =
        sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES('duplicate-user',$1)")
            .bind(canonical.id)
            .execute(&pool)
            .await
            .unwrap_err();
    assert!(
        matches!(duplicate,sqlx::Error::Database(ref error) if error.constraint()==Some("user_identities_user_id_key"))
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM user_identities WHERE user_id=$1")
        .bind(other.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let duplicate =
        sqlx::query("INSERT INTO user_identities(subject,user_id) VALUES('same-subject',$1)")
            .bind(other.id)
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert!(
        matches!(duplicate,sqlx::Error::Database(ref error) if error.constraint()==Some("user_identities_pkey"))
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(canonical.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    global.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn identity_sessions_are_private_revocable_and_separate_from_password_sessions(
    pool: sqlx::PgPool,
) {
    let sessions = Sessions::new(database(&pool).await);
    let legacy = sessions
        .register("ursus95@gmail.com", "fixture-browser-password".into())
        .await
        .unwrap();
    let user = sessions
        .link_identity("session-subject", "ursus95@gmail.com")
        .await
        .unwrap();
    assert!(
        sessions
            .create_identity_session("missing")
            .await
            .unwrap()
            .is_none()
    );
    let first = sessions
        .create_identity_session("session-subject")
        .await
        .unwrap()
        .unwrap();
    let second = sessions
        .create_identity_session("session-subject")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.expose_secret(), second.expose_secret());
    assert!(
        sessions
            .user(first.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .identity_user(legacy.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    for invalid in ["", "bad", &"!".repeat(43)] {
        assert!(sessions.identity_user(invalid).await.unwrap().is_none());
    }
    sqlx::query("UPDATE users SET admin=false WHERE id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    let current = sessions
        .identity_user(first.expose_secret())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, user.id);
    assert_eq!(current.tenant_id, user.tenant_id);
    assert!(!current.admin);
    sessions.logout(first.expose_secret()).await.unwrap();
    assert!(
        sessions
            .identity_user(first.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    sessions
        .logout_identity(legacy.expose_secret())
        .await
        .unwrap();
    assert!(
        sessions
            .user(legacy.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    sessions
        .logout_identity(first.expose_secret())
        .await
        .unwrap();
    assert!(
        sessions
            .identity_user(first.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .identity_user(second.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE identity_sessions SET expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sessions
            .identity_user(second.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    sessions.sweep().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let third = sessions
        .create_identity_session("session-subject")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("DELETE FROM user_identities WHERE subject='session-subject'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sessions
            .identity_user(third.expose_secret())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .user(legacy.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
