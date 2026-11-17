#[path = "../../src/access.rs"]
mod access;
mod access_enforcement;
#[path = "../../src/admin.rs"]
mod admin;
mod authorization;
mod browser;
mod endpoints;
#[path = "../../src/identity.rs"]
mod identity;
mod identity_login;
#[path = "../../src/mcp_admin.rs"]
mod mcp_admin;
#[path = "../../src/oauth.rs"]
mod oauth;
#[path = "../../src/server.rs"]
mod server;
mod upstream_accounts;
#[path = "../../src/webui.rs"]
mod webui;
use sqlx::ConnectOptions;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[sqlx::test(migrations = "../core/migrations")]
async fn token_cli_stdout_is_machine_readable(pool: sqlx::PgPool) {
    let database = pool.connect_options().to_url_lossy();
    let call = |args: &[&str]| {
        let binary = std::env::var_os("CODEMODE_GATEWAY_BIN")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_codemode").into());
        let mut command = Command::new(binary);
        command
            .env("CODEMODE_DATABASE_URL", database.as_str())
            .env("CODEMODE_LOG", "info")
            .args(args);
        command
    };
    let created = call(&["token", "create", "stdout-regression"])
        .output()
        .await
        .unwrap();
    assert!(created.status.success());
    let text = String::from_utf8(created.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.trim().starts_with("cm_") && text.trim().len() == 46);
    let listed = call(&["token", "list"]).output().await.unwrap();
    assert!(listed.status.success());
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows.len(), 1);
    let id = rows[0]["id"].as_str().unwrap();
    let revoked = call(&["token", "revoke", id]).output().await.unwrap();
    assert!(revoked.status.success());
    assert_eq!(revoked.stdout, b"revoked\n");
    let mut hashing = call(&["password", "hash"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hashing
        .stdin
        .take()
        .unwrap()
        .write_all(b"fixture-browser-password\n")
        .await
        .unwrap();
    let hashed = hashing.wait_with_output().await.unwrap();
    assert!(hashed.status.success());
    let hash = String::from_utf8(hashed.stdout).unwrap();
    assert_eq!(hash.lines().count(), 1);
    assert!(hash.starts_with("$argon2id$v=19$"));
    assert!(!hash.contains("fixture-browser-password"));
    assert!(!String::from_utf8_lossy(&hashed.stderr).contains("fixture-browser-password"));
}
