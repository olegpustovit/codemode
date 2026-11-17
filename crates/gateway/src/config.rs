use anyhow::{Context, bail};
use codemode_core::components::execution::Limits;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

pub struct Config {
    pub identity: Option<idclient::Config>,
    pub identity_access_enabled: bool,
    pub bind: SocketAddr,
    pub hostname: String,
    pub database_url: secrecy::SecretString,
    pub sources_file: PathBuf,
    pub monty_bin: PathBuf,
    pub encryption_key: Option<secrecy::SecretString>,
    pub encryption_key_id: String,
    pub log_json: bool,
    pub execution_limits: Limits,
    pub output_bytes: usize,
    pub session_requests_per_minute: u32,
    pub token_requests_per_minute: u32,
    pub endpoint_requests_per_minute: u32,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let hostname = std::env::var("CODEMODE_HOSTNAME")
            .unwrap_or_else(|_| "codemode.microapps.space".into())
            .to_ascii_lowercase();
        if hostname.len() > 253
            || hostname.split('.').any(|s| {
                s.is_empty()
                    || s.len() > 63
                    || s.starts_with('-')
                    || s.ends_with('-')
                    || !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            bail!("CODEMODE_HOSTNAME must be a DNS hostname without a scheme or port");
        }
        let config = Self {
            identity: crate::identity::configuration(&hostname)?,
            identity_access_enabled: std::env::var("CODEMODE_ID_ACCESS_ENABLED")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .context("CODEMODE_ID_ACCESS_ENABLED must be true or false")?,
            bind: std::env::var("CODEMODE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:2093".into())
                .parse()
                .context("CODEMODE_BIND")?,
            hostname,
            database_url: std::env::var("CODEMODE_DATABASE_URL")
                .context("CODEMODE_DATABASE_URL is required")?
                .into(),
            sources_file: std::env::var_os("CODEMODE_SOURCES_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("sources.toml")),
            monty_bin: std::env::var_os("CODEMODE_MONTY_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("./monty")),
            encryption_key: std::env::var("CODEMODE_ENCRYPTION_KEY")
                .ok()
                .map(Into::into),
            encryption_key_id: std::env::var("CODEMODE_ENCRYPTION_KEY_ID")
                .unwrap_or_else(|_| "primary".into()),
            log_json: std::env::var("CODEMODE_LOG_JSON")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .context("CODEMODE_LOG_JSON must be true or false")?,
            execution_limits: Limits {
                result_bytes: limit("CODEMODE_RESULT_BYTES", 32768, 256..=1048576)?,
                concurrent_executions: limit("CODEMODE_CONCURRENT_EXECUTIONS", 8, 1..=64)?,
                concurrent_calls: limit("CODEMODE_CONCURRENT_CALLS", 16, 1..=100)?,
                upstream_timeout: Duration::from_secs(limit(
                    "CODEMODE_UPSTREAM_TIMEOUT_SECS",
                    30,
                    1..=60,
                )? as u64),
            },
            output_bytes: limit("CODEMODE_OUTPUT_BYTES", 32768, 256..=1048576)?,
            session_requests_per_minute: limit(
                "CODEMODE_SESSION_REQUESTS_PER_MINUTE",
                600,
                1..=60_000,
            )? as u32,
            token_requests_per_minute: limit(
                "CODEMODE_TOKEN_REQUESTS_PER_MINUTE",
                6000,
                1..=60_000,
            )? as u32,
            endpoint_requests_per_minute: limit(
                "CODEMODE_ENDPOINT_REQUESTS_PER_MINUTE",
                6000,
                1..=600_000,
            )? as u32,
        };
        if !config.bind.ip().is_loopback() {
            bail!("CODEMODE_BIND must use loopback; expose the gateway through Caddy");
        }
        if config.identity_access_enabled && config.identity.is_none() {
            bail!(
                "CODEMODE_ID_ACCESS_ENABLED requires CODEMODE_ID_ENABLED and its confidential client"
            );
        }
        Ok(config)
    }
}

fn limit(
    name: &str,
    default: usize,
    range: std::ops::RangeInclusive<usize>,
) -> anyhow::Result<usize> {
    let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
    parse_limit(&value, range).with_context(|| format!("{name} is outside its supported range"))
}

fn parse_limit(value: &str, range: std::ops::RangeInclusive<usize>) -> anyhow::Result<usize> {
    let value = value.parse()?;
    anyhow::ensure!(range.contains(&value), "invalid limit");
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_reject_zero_overflow_and_invalid_numbers() {
        for value in ["0", "65", "-1", "abc", "999999999999999999999999"] {
            assert!(parse_limit(value, 1..=64).is_err());
        }
        assert_eq!(parse_limit("8", 1..=64).unwrap(), 8);
    }
}
