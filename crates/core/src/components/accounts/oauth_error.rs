use super::Error;

#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OAuthStage {
    Probe,
    ResourceMetadata,
    ServerMetadata,
    Registration,
    Consent,
    Exchange,
    Store,
    Refresh,
}
impl std::fmt::Display for OAuthStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Probe => "step 1 (probe MCP)",
            Self::ResourceMetadata => "step 2 (protected-resource metadata)",
            Self::ServerMetadata => "step 3 (authorization-server metadata)",
            Self::Registration => "step 4 (client registration)",
            Self::Consent => "step 5 (browser authorization)",
            Self::Exchange => "step 6 (token exchange)",
            Self::Store => "step 7 (credential storage)",
            Self::Refresh => "token refresh",
        })
    }
}
impl OAuthStage {
    pub fn recovery(self) -> &'static str {
        match self {
            Self::Probe | Self::ResourceMetadata | Self::ServerMetadata => {
                "check the server address and its OAuth support, or link an API token"
            }
            Self::Registration => {
                "configure a provider OAuth app or link an API token if automatic registration is unsupported"
            }
            Self::Consent | Self::Exchange => {
                "retry linking the account; check the provider app's redirect URI and scopes if it fails again"
            }
            Self::Store => {
                "retry linking the account; contact the operator if credentials cannot be stored"
            }
            Self::Refresh => {
                "retry the call; link the account again if the provider rejected its refresh token"
            }
        }
    }
}

// Scrub before constructing the error: Debug, HTTP and background logs share this value.
pub(super) fn failure(stage: OAuthStage, error: impl std::fmt::Display, secrets: &[&str]) -> Error {
    let mut detail = error.to_string();
    if detail.len() > 16_384 {
        return Error::OAuth {
            stage,
            detail: "provider error exceeded the 16 KB diagnostic limit".into(),
        };
    }
    let mut secrets: Vec<_> = secrets.iter().copied().filter(|s| !s.is_empty()).collect();
    secrets.sort_unstable_by_key(|s| std::cmp::Reverse(s.len()));
    for secret in secrets {
        redact(&mut detail, secret);
    }
    // Registration errors can contain a provider JSON body, including credentials we never accepted.
    if let Some(start) = detail.find('{')
        && let Ok(mut body) = serde_json::from_str::<serde_json::Value>(&detail[start..])
    {
        let mut returned_secrets = Vec::new();
        scrub(&mut body, &mut returned_secrets);
        detail.replace_range(start.., &body.to_string());
        returned_secrets.sort_unstable_by_key(|secret| std::cmp::Reverse(secret.len()));
        for secret in returned_secrets {
            redact(&mut detail, &secret);
        }
    }
    let detail: String = detail
        .chars()
        .filter(|c| !c.is_control())
        .take(1_024)
        .collect();
    Error::OAuth { stage, detail }
}
fn redact(message: &mut String, secret: &str) {
    if secret.is_empty() {
        return;
    }
    *message = message.replace(secret, "[REDACTED]");
    let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
    *message = message.replace(&encoded, "[REDACTED]");
    let escaped = serde_json::to_string(secret).unwrap_or_default();
    if let Some(escaped) = escaped.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        *message = message.replace(escaped, "[REDACTED]");
    }
}
fn scrub(value: &mut serde_json::Value, secrets: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "access_token"
                        | "refresh_token"
                        | "id_token"
                        | "client_secret"
                        | "code"
                        | "code_verifier"
                        | "authorization"
                ) {
                    if let Some(secret) = value.as_str() {
                        secrets.push(secret.to_owned());
                    }
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    scrub(value, secrets);
                }
            }
        }
        serde_json::Value::Array(values) => {
            values.iter_mut().for_each(|value| scrub(value, secrets))
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_failures_keep_stage_and_provider_reason_but_not_credentials() {
        let error = failure(
            OAuthStage::Exchange,
            "invalid_grant: code=sensitive/code secret=sensitive\n",
            &["sensitive", "sensitive/code"],
        );
        assert!(matches!(
            error,
            Error::OAuth {
                stage: OAuthStage::Exchange,
                ..
            }
        ));
        let text = format!("{error:?}");
        assert!(!text.contains("sensitive"));
        assert!(text.contains("invalid_grant"));
        let error = failure(
            OAuthStage::Registration,
            r#"HTTP 400: {"error":"invalid_client_metadata","client_secret":"never-accepted","error_description":"do not log never-accepted","nested":{"access_token":"also-secret"}}"#,
            &[],
        );
        let text = format!("{error:?}");
        assert!(!text.contains("never-accepted"));
        assert!(!text.contains("also-secret"));
        assert!(text.contains("invalid_client_metadata"));
        let Error::OAuth { detail, .. } = failure(OAuthStage::Probe, "é".repeat(2_000), &[])
        else {
            panic!()
        };
        assert_eq!(detail.chars().count(), 1_024);
        let error = failure(
            OAuthStage::Registration,
            "oversized-secret".repeat(2_000),
            &[],
        );
        assert!(!format!("{error:?}").contains("oversized-secret"));
        assert!(error.to_string().contains("diagnostic limit"));
        assert!(
            !failure(OAuthStage::Exchange, "secret%2Fcode", &["secret/code"])
                .to_string()
                .contains("secret")
        );
    }
}
