mod access;
mod admin;
mod bootstrap;
mod config;
mod identity;
mod mcp_admin;
mod oauth;
mod server;
mod webui;

use tracing_subscriber::fmt::writer::MakeWriterExt;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    let config = match config::Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration: {error:#}");
            return std::process::ExitCode::from(2);
        }
    };
    let filter = logging_filter(
        tracing_subscriber::EnvFilter::try_from_env("CODEMODE_LOG")
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    );
    let cli = std::env::args().nth(1).is_some();
    let logging = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(
            std::io::stderr
                .with_filter(move |_| cli)
                .or_else(std::io::stdout),
        );
    if config.log_json {
        logging.json().init();
    } else {
        logging.init();
    }
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| runtime.block_on(bootstrap::run(config)));
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error=%format!("{error:#}"),"fatal");
            std::process::ExitCode::FAILURE
        }
    }
}

fn logging_filter(mut filter: tracing_subscriber::EnvFilter) -> tracing_subscriber::EnvFilter {
    // OAuth events and HTTP traces can contain credentials in bodies, headers or query strings.
    for directive in [
        "rmcp::transport::auth=off",
        "reqwest=off",
        "hyper=off",
        "hyper_util=off",
        "h2=off",
    ] {
        filter = filter.add_directive(directive.parse().expect("static tracing directive"));
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn trace_logging_suppresses_oauth_and_http_credentials_and_keeps_application_events() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_env_filter(logging_filter(tracing_subscriber::EnvFilter::new("trace")))
            .with_writer(move || Buffer(sink.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target:"rmcp::transport::auth",code="fixture-authorization-code","token exchange");
            tracing::warn!(target:"rmcp::transport::auth",response="fixture-refresh-token","refresh rejected");
            tracing::trace!(target:"reqwest::connect",uri="https://api.example/?key=fixture-query-credential","HTTP request");
            tracing::trace!(target:"hyper::proto::h1",body="fixture-query-credential","HTTP request");
            tracing::trace!(target:"hyper_util::client",uri="fixture-query-credential","HTTP request");
            tracing::trace!(target:"h2::codec",header="fixture-query-credential","HTTP request");
            tracing::debug!(target:"codemode","application event retained");
        });
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.contains("application event retained"));
        assert!(!output.contains("fixture-authorization-code"));
        assert!(!output.contains("fixture-refresh-token"));
        assert!(!output.contains("fixture-query-credential"));
    }
}
