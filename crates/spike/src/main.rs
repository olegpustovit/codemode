use std::{path::PathBuf, time::Duration};

use anyhow::{Context, bail};
use codemode_spike::{
    sandbox::{MontySandbox, Sandbox, Step, dispatch_stub, execute},
    server,
};
use serde_json::value::RawValue;
use tokio_util::sync::CancellationToken;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "serve") {
        let bind = std::env::var("CODEMODE_BIND").unwrap_or_else(|_| "127.0.0.1:2093".into());
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .context("bind spike listener")?;
        let stop = CancellationToken::new();
        let shutdown = stop.clone();
        axum::serve(listener, server::router(stop))
            .with_graceful_shutdown(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    shutdown.cancel();
                }
            })
            .await?;
        return Ok(());
    }
    let binary =
        PathBuf::from(std::env::var("CODEMODE_MONTY_BIN").context("set CODEMODE_MONTY_BIN")?);
    let key_file = std::env::var("CODEMODE_SNAPSHOT_KEY_FILE")
        .context("set CODEMODE_SNAPSHOT_KEY_FILE (32 binary bytes)")?;
    let key: [u8; 32] = std::fs::read(key_file)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("snapshot key must be 32 bytes"))?;
    let mut sandbox = MontySandbox::new(&binary, Duration::from_secs(1), key).await?;
    match args.get(1).map(String::as_str) {
        Some("trial-discover" | "trial-run") => {
            let config = PathBuf::from(args.get(2).context("trial requires a sources file")?);
            let output = PathBuf::from(args.get(3).context("trial requires an output path")?);
            let tasks = args.get(4).map(PathBuf::from);
            codemode_spike::trial::run(&config, tasks.as_deref(), &output, &mut sandbox).await?;
        }
        Some("run") => {
            let path = args.get(2).context("run requires a Python file")?;
            println!(
                "{:?}",
                execute(&sandbox, &std::fs::read_to_string(path)?).await?
            );
        }
        Some("snapshot-write") => {
            let path = args.get(2).context("snapshot-write requires a path")?;
            let (run, step) = sandbox
                .start(
                    "print('before')\na = await tools.stub.echo(value=21)\nreturn a['value'] * 2",
                )
                .await?;
            let Step::Calls(calls) = step else {
                bail!("expected pending call")
            };
            anyhow::ensure!(calls.len() == 1, "expected one call");
            std::fs::write(path, sandbox.snapshot(run).await?)?;
            println!("snapshot written; caller must store pending calls alongside it");
        }
        Some("snapshot-read") => {
            let path = args.get(2).context("snapshot-read requires a path")?;
            let (run, _) = sandbox.restore(&std::fs::read(path)?).await?;
            // This phase-0 fixture's host retained the pending call and id 0.
            let result = RawValue::from_string("{\"value\":21}".into())?;
            let (_, step) = sandbox.resume(run, vec![(0, Ok(result))]).await?;
            match step {
                Step::Done { result, output } => {
                    anyhow::ensure!(result.get() == "42", "wrong restored result");
                    anyhow::ensure!(output == "before\n", "wrong captured output");
                    println!("restored in a new gateway process: result=42; output=before");
                }
                Step::Calls(calls) => {
                    let _ = dispatch_stub(calls).await;
                    bail!("unexpected extra calls")
                }
            }
        }
        _ => bail!("usage: codemode-spike serve|run FILE|snapshot-write FILE|snapshot-read FILE"),
    }
    sandbox.pool.close().await;
    Ok(())
}
