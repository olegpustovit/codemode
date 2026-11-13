use std::{collections::BTreeMap, path::Path, time::Duration};

use anyhow::{Context, ensure};
use futures_util::{StreamExt, stream};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion},
    service::{Peer, RoleClient},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};

use crate::sandbox::{CallResult, MontySandbox, Sandbox, Step};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    slug: String,
    url: String,
    token_file: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Task {
    name: String,
    code: String,
}

pub async fn run(
    config: &Path,
    tasks: Option<&Path>,
    output: &Path,
    sandbox: &mut MontySandbox,
) -> anyhow::Result<()> {
    let sources: Vec<Source> = serde_json::from_slice(&std::fs::read(config)?)?;
    let mut connections = Vec::new();
    let mut peers: BTreeMap<String, Peer<RoleClient>> = BTreeMap::new();
    let mut prelude = Vec::new();
    let mut schemas = BTreeMap::new();
    for source in sources {
        ensure!(
            source.url.starts_with("http://127.0.0.1:"),
            "trial only accepts loopback fixtures"
        );
        let token = std::fs::read_to_string(&source.token_file)
            .context("read test credential reference")?;
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(source.url).auth_header(token.trim()),
        );
        let client = tokio::time::timeout(
            Duration::from_secs(10),
            ClientConfig::default()
                .with_protocol_version(ProtocolVersion::V_2025_11_25)
                .serve(transport),
        )
        .await
        .context("connect test source timeout")?
        .context("connect test source")?;
        let tools = client.list_all_tools().await?;
        let names = tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>();
        prelude.push((source.slug.clone(), names));
        schemas.insert(source.slug.clone(), serde_json::to_value(&tools)?);
        peers.insert(source.slug, client.peer().clone());
        connections.push(client);
    }
    sandbox.set_trial_tools(&prelude)?;
    if let Some(tasks) = tasks {
        let tasks: Vec<Task> = serde_json::from_slice(&std::fs::read(tasks)?)?;
        let mut report = Vec::new();
        for task in tasks {
            let start = std::time::Instant::now();
            let outcome = tokio::time::timeout(Duration::from_secs(60), async {
                let (mut run, mut step) = sandbox.start(&task.code).await?;
                loop {
                    match step {
                        Step::Done { result, output } => {
                            return Ok::<_, crate::sandbox::Error>((result, output));
                        }
                        Step::Calls(calls) => {
                            let results = stream::iter(calls)
                                .map(|call| {
                                    let peers = &peers;
                                    async move {
                                        let result =
                                            call_upstream(peers, &call.path, &call.args).await;
                                        (call.id, result)
                                    }
                                })
                                .buffer_unordered(16)
                                .collect()
                                .await;
                            (run, step) = sandbox.resume(run, results).await?;
                        }
                    }
                }
            })
            .await;
            let (success, result, printed, error) = match outcome {
                Ok(Ok((result, printed))) => (
                    true,
                    serde_json::from_str::<Value>(result.get())?,
                    printed,
                    None,
                ),
                Ok(Err(err)) => (false, Value::Null, String::new(), Some(err.to_string())),
                Err(_) => (
                    false,
                    Value::Null,
                    String::new(),
                    Some("60s deadline".into()),
                ),
            };
            println!(
                "{}: {} ({:.1} ms)",
                task.name,
                if success { "completed" } else { "failed" },
                start.elapsed().as_secs_f64() * 1000.0
            );
            report.push(serde_json::json!({"name":task.name,"code":task.code,"success":success,"result":result,"output":printed,"error":error,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0}));
        }
        std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    } else {
        std::fs::write(output, serde_json::to_vec_pretty(&schemas)?)?;
        println!("Saved real tool schemas for {} test sources", peers.len());
    }
    for client in connections {
        client.cancel().await?;
    }
    Ok(())
}

async fn call_upstream(
    peers: &BTreeMap<String, Peer<RoleClient>>,
    path: &str,
    args: &RawValue,
) -> CallResult {
    let (source, name) = path
        .split_once('.')
        .ok_or_else(|| "invalid tool path".to_owned())?;
    let peer = peers
        .get(source)
        .ok_or_else(|| "unknown source".to_owned())?;
    // shortcut: the spike parses args to satisfy rmcp; Phase 1 validates them against the saved schema.
    let args = serde_json::from_str(args.get()).map_err(|e| format!("invalid args: {e}"))?;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        peer.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(args)),
    )
    .await
    .map_err(|_| "upstream timeout".to_owned())?
    .map_err(|e| e.to_string())?;
    if result.is_error == Some(true) {
        return Err(result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    if let Some(content) = result.structured_content {
        return serde_json::value::to_raw_value(&content).map_err(|e| e.to_string());
    }
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    match RawValue::from_string(text.clone()) {
        Ok(value) => Ok(value),
        Err(_) => serde_json::value::to_raw_value(&text).map_err(|e| e.to_string()),
    }
}
