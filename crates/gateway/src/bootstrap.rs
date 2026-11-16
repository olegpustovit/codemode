use crate::{
    config::Config,
    server::{self, State},
};
use anyhow::Context;
use codemode_core::{
    components::{
        accounts::Accounts, auth::Auth, catalogue::Catalogue, execution::Execution,
        sessions::Sessions, sources::Sources, workspaces::Workspaces,
    },
    infrastructure::{postgres::Database, sandbox::MontySandbox},
};
use secrecy::ExposeSecret;
use std::{sync::Arc, time::Duration};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub async fn run(config: Config) -> anyhow::Result<()> {
    let db = Database::connect(config.database_url.expose_secret())
        .await
        .context("connect database and apply migrations")?;
    let auth = Auth::new(db.clone());
    let endpoints = Workspaces::new(db.clone(), config.hostname.clone())
        .context("configure workspace service")?;
    endpoints
        .initialize()
        .await
        .context("initialize main endpoint")?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["password", "hash"] {
        let hash = tokio::task::spawn_blocking(|| {
            use secrecy::{ExposeSecret, SecretString};
            use std::io::Read;
            let mut password = String::new();
            std::io::stdin()
                .take(1026)
                .read_to_string(&mut password)
                .context("read administrator password from stdin")?;
            let password: SecretString = password.into();
            codemode_core::components::sessions::hash_password(
                password.expose_secret().trim_end_matches(['\r', '\n']),
            )
            .context("password must contain 12..1024 bytes")
        })
        .await??;
        println!("{hash}");
        db.close().await;
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "token") {
        match args.get(1).map(String::as_str) {
            Some("create") if args.len() == 3 => {
                let (id, token) = auth.create(&args[2], &config.hostname).await?;
                eprintln!("Created token {id}; save this value now. It will not be shown again.");
                println!("{token}");
            }
            Some("list") if args.len() == 2 => {
                println!("{}", serde_json::to_string_pretty(&auth.list().await?)?)
            }
            Some("revoke") if args.len() == 3 => {
                anyhow::ensure!(
                    auth.revoke(args[2].parse().context("token UUID")?).await?,
                    "token does not exist or is already revoked"
                );
                println!("revoked");
            }
            _ => {
                anyhow::bail!("usage: codemode token create NAME | token list | token revoke UUID")
            }
        }
        db.close().await;
        return Ok(());
    }
    anyhow::ensure!(
        args.is_empty(),
        "usage: codemode [token create NAME | token list | token revoke UUID | password hash]"
    );
    let encryption_key = config
        .encryption_key
        .as_ref()
        .map(|k| decode_key(k.expose_secret()))
        .transpose()
        .context("CODEMODE_ENCRYPTION_KEY must contain 64 hex characters")?;
    let accounts = Accounts::new(db.clone(), encryption_key, config.encryption_key_id)
        .context("configure linked-account encryption")?;
    let sessions = Sessions::new(db.clone());
    let identity_routes = match config.identity {
        Some(identity) => Some(
            crate::identity::router(identity, sessions.clone(), config.hostname.clone())
                .await
                .context("configure Identity login and discover issuer")?,
        ),
        None => None,
    };
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let oauth = Arc::new(codemode_core::components::auth::oauth::OAuth::new(
        db.clone(),
        config.hostname.clone(),
    ));
    let oauth_sweeper = oauth.clone();
    let session_sweeper = sessions.clone();
    let account_sweeper = accounts.clone();
    let session_stop = stop.clone();
    tasks.spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(300));
        loop {
            tokio::select! { biased; _=session_stop.cancelled()=>break, _=timer.tick()=>{} }
            if let Err(error) = session_sweeper.sweep().await {
                tracing::error!(error=%error,"session expiry cleanup failed");
            }
            if let Err(error) = oauth_sweeper.sweep().await {
                tracing::error!(error=?error,"OAuth expiry cleanup failed");
            }
            if let Err(error) = account_sweeper.sweep_oauth().await {
                tracing::error!(error=?error,"upstream authorization expiry cleanup failed");
            }
        }
    });
    let history = codemode_core::components::agent_sessions::AgentSessions::new(db.clone());
    let history_stop = stop.clone();
    tasks.spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {biased; _=history_stop.cancelled()=>break,_=timer.tick()=>{}}
            if let Err(error) = history.sweep().await {
                tracing::error!(error=?error,"agent history retention cleanup failed");
            }
        }
    });
    let catalogue = Catalogue::new(db.clone());
    let sources = Sources::new(
        db.clone(),
        catalogue.clone(),
        accounts.clone(),
        endpoints.clone(),
    );
    sources
        .import_file(&config.sources_file)
        .await
        .context("import initial sources")?;
    let sandbox = Arc::new(
        MontySandbox::new(
            &config.monty_bin,
            Duration::from_secs(60),
            config.output_bytes,
        )
        .await
        .context("start Monty worker pool")?,
    );
    let execution = Arc::new(Execution::new(
        db.clone(),
        sources.clone(),
        sandbox.clone(),
        tasks.clone(),
        stop.clone(),
        config.execution_limits,
    ));
    execution
        .recover()
        .await
        .context("recover interrupted executions")?;
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .context("bind HTTP listener")?;
    tracing::info!(address=%listener.local_addr()?,hostname=%config.hostname,"Codemode listening");
    let mcp_tasks = server::Tasks::default();
    let state = State {
        mcp_tasks: mcp_tasks.clone(),
        rate_limit: Arc::new(
            codemode_core::components::auth::rate_limit::RateLimiter::new(
                config.token_requests_per_minute,
                config.endpoint_requests_per_minute,
            ),
        ),
        db: db.clone(),
        auth,
        catalogue,
        execution,
        hostname: config.hostname,
        sessions: Arc::new(sessions),
        identity_enabled: identity_routes.is_some(),
        identity_routes,
        accounts: Arc::new(accounts),
        sources: Arc::new(sources),
        workspaces: Arc::new(endpoints),
        stop: stop.clone(),
        oauth,
    };
    let router = server::router(state);
    let signal_stop = stop.clone();
    let serving=axum::serve(listener,router).with_graceful_shutdown(async move {
        let mut term=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _=tokio::signal::ctrl_c()=>{},
            _=async {if let Some(signal)=term.as_mut(){signal.recv().await;}else{std::future::pending::<()>().await;}}=>{},
            _=signal_stop.cancelled()=>{},
        }
        signal_stop.cancel();
    }).await;
    stop.cancel();
    mcp_tasks.shutdown();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
    db.close().await;
    serving.context("serve HTTP")
}

fn decode_key(text: &str) -> anyhow::Result<[u8; 32]> {
    anyhow::ensure!(text.len() == 64, "key must contain 64 hex characters");
    let mut key = [0; 32];
    for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let high = (pair[0] as char)
            .to_digit(16)
            .context("invalid hexadecimal key")?;
        let low = (pair[1] as char)
            .to_digit(16)
            .context("invalid hexadecimal key")?;
        key[i] = (high * 16 + low) as u8;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_requires_exactly_32_bytes() {
        assert_eq!(decode_key(&"ab".repeat(32)).unwrap(), [0xab; 32]);
        assert!(decode_key(&"z".repeat(64)).is_err());
        assert!(decode_key("01").is_err());
    }
}
