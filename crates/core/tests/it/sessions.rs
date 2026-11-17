use super::*;
use codemode_core::components::sessions::{Error, Sessions, hash_password};
use secrecy::ExposeSecret;

#[sqlx::test(migrations = "./migrations")]
async fn browser_sessions_are_hashed_expiring_revocable_and_password_bound(pool: sqlx::PgPool) {
    let db = database(&pool).await;
    let sessions = Sessions::new(db);
    assert!(sessions.first_user().await.unwrap());
    let token = sessions
        .register(" Ursus95@GMAIL.com ", "fixture-browser-password".into())
        .await
        .unwrap();
    let user = sessions.user(token.expose_secret()).await.unwrap().unwrap();
    assert!(user.admin);
    assert_eq!(user.email, "ursus95@gmail.com");
    assert!(!sessions.first_user().await.unwrap());
    assert!(
        sessions
            .login("ursus95@gmail.com", "wrong".into())
            .await
            .unwrap()
            .is_none()
    );
    let token = token.expose_secret();
    assert!(sessions.authenticate(token).await.unwrap());
    assert!(!sessions.authenticate(&format!("cm_{token}")).await.unwrap());
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, Sha256::digest(token.as_bytes()).to_vec());
    let hash = hash_password("different-browser-password").unwrap();
    sqlx::query("UPDATE users SET password_hash=$1")
        .bind(hash)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!sessions.authenticate(token).await.unwrap());
    let token = sessions
        .login("URSUS95@GMAIL.COM", "different-browser-password".into())
        .await
        .unwrap()
        .unwrap();
    assert!(sessions.authenticate(token.expose_secret()).await.unwrap());
    sqlx::query("UPDATE sessions SET expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!sessions.authenticate(token.expose_secret()).await.unwrap());
    sessions.sweep().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let token = sessions
        .login("ursus95@gmail.com", "different-browser-password".into())
        .await
        .unwrap()
        .unwrap();
    sessions.logout(token.expose_secret()).await.unwrap();
    assert!(!sessions.authenticate(token.expose_secret()).await.unwrap());
}

#[sqlx::test(migrations = "./migrations")]
async fn browser_login_attempts_are_bounded(pool: sqlx::PgPool) {
    let sessions = Sessions::new(database(&pool).await);
    for _ in 0..30 {
        assert!(matches!(
            sessions.register("not-an-email", "anything".into()).await,
            Err(Error::NotAllowed)
        ));
    }
    assert!(matches!(
        sessions
            .login("ursus95@gmail.com", "fixture-browser-password".into())
            .await,
        Err(Error::Busy)
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn first_registration_is_atomic_and_invites_cannot_be_spoofed(pool: sqlx::PgPool) {
    let sessions = Sessions::new(database(&pool).await);
    for email in [
        "",
        "@olegpustovit.com",
        "alice@evil@olegpustovit.com",
        "alice\n@olegpustovit.com",
        "a..b@olegpustovit.com",
    ] {
        assert!(
            matches!(
                sessions
                    .register(email, "fixture-browser-password".into())
                    .await,
                Err(Error::NotAllowed)
            ),
            "{email}"
        );
    }
    let (a, b) = tokio::join!(
        sessions.register("ursus95@gmail.com", "fixture-browser-password".into()),
        sessions.register("alice@olegpustovit.com", "fixture-browser-password".into())
    );
    let a = sessions
        .user(a.unwrap().expose_secret())
        .await
        .unwrap()
        .unwrap();
    let b = sessions
        .user(b.unwrap().expose_secret())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(a.tenant_id, b.tenant_id);
    assert_ne!(a.admin, b.admin);
    assert_eq!(if a.admin { a.tenant_id } else { b.tenant_id }, Uuid::nil());
    assert!(matches!(
        sessions
            .register("ALICE@OLEGPUSTOVIT.COM", "fixture-browser-password".into())
            .await,
        Err(Error::Exists)
    ));
    assert!(
        sessions
            .register("bob@olegpustovit.com", "short".into())
            .await
            .is_err()
    );
}
