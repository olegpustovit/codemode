pub use super::providers::{Entry, Method, registry};
use super::sources::{Source, Sources, Transport};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    #[serde(flatten)]
    pub entry: Entry,
    pub source_id: Option<Uuid>,
    pub state: String,
    pub tool_count: i64,
    pub tool_names: Vec<String>,
    pub error: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("integration not found")]
    NotFound,
    #[error(transparent)]
    Registry(#[from] super::providers::Error),
    #[error(transparent)]
    Source(#[from] super::sources::Error),
}
pub async fn list(managed: &Sources) -> Result<Vec<Integration>, Error> {
    let sources = managed.list().await?;
    let mut result = Vec::new();
    let mut matched = std::collections::BTreeSet::new();
    for entry in registry()? {
        let source = sources.iter().find(|record| matches!(&record.source.transport, Transport::McpHttp { url, .. } if url.trim_end_matches('/') == entry.url.trim_end_matches('/')));
        if let Some(source) = source {
            matched.insert(source.id);
        }
        result.push(Integration {
            source_id: source.map(|s| s.id),
            state: source.map_or("available", |s| s.status.as_str()).into(),
            tool_count: source.map_or(0, |s| s.tool_count),
            tool_names: match source {
                Some(source) if source.status == "ready" => managed.tool_names(source.id).await,
                _ => Vec::new(),
            },
            error: source.and_then(|s| s.last_error.clone()),
            entry,
        });
    }
    for source in sources
        .into_iter()
        .filter(|source| !matched.contains(&source.id))
    {
        let local = matches!(source.source.transport, Transport::McpStdio { .. });
        result.push(Integration {
            tool_names: if source.status == "ready" {
                managed.tool_names(source.id).await
            } else {
                Vec::new()
            },
            entry: Entry {
                slug: format!("source-{}", source.id),
                name: source.name,
                category: if local { "Local" } else { "Custom" }.into(),
                description: match source.source.transport {
                    Transport::McpHttp { .. } => "Custom MCP server",
                    Transport::McpStdio { .. } => "Operator-managed local server",
                    Transport::OpenApi { .. } => "OpenAPI source",
                    Transport::Graphql { .. } => "GraphQL source",
                }
                .into(),
                // Custom connection configuration is available only through the browser source API.
                url: String::new(),
                docs: String::new(),
                icon: source.icon,
                oauth: None,
                methods: if local {
                    vec![Method::Local]
                } else if source.source.requires_account {
                    if matches!(source.source.transport, Transport::McpHttp { .. }) {
                        vec![Method::OneClick, Method::ApiKey]
                    } else {
                        vec![Method::ApiKey]
                    }
                } else {
                    vec![Method::Open]
                },
            },
            source_id: Some(source.id),
            state: source.status,
            tool_count: source.tool_count,
            error: source.last_error,
        });
    }
    Ok(result)
}

pub async fn add(sources: &Sources, slug: &str) -> Result<Uuid, Error> {
    let entry = registry()?
        .into_iter()
        .find(|entry| entry.slug == slug)
        .ok_or(Error::NotFound)?;
    // Repeating a browser click reuses the user's existing connection to this vendor.
    if let Some(existing) = sources.list().await?.into_iter().find(|record| matches!(&record.source.transport, Transport::McpHttp { url, .. } if url.trim_end_matches('/') == entry.url.trim_end_matches('/'))) {
        return Ok(existing.id);
    }
    Ok(sources
        .save(
            None,
            Source {
                slug: entry.slug,
                requires_account: !entry.methods.contains(&Method::Open),
                transport: Transport::McpHttp {
                    url: entry.url,
                    bearer_env: None,
                },
            },
        )
        .await?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_uses_unique_lowercase_names_and_safe_remote_addresses() {
        let entries = registry().unwrap();
        let mut slugs = std::collections::BTreeSet::new();
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().all(|entry| entry.slug != "sentry"));
        for entry in entries {
            assert!(slugs.insert(entry.slug.clone()));
            assert!(
                entry
                    .slug
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            );
            assert!(!entry.methods.is_empty());
            for address in [&entry.url, &entry.docs] {
                let url = url::Url::parse(address).unwrap();
                assert_eq!(url.scheme(), "https");
                assert!(
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                );
            }
            assert_eq!(
                entry.methods.contains(&Method::OAuthApp),
                entry.oauth.is_some()
            );
            if let Some(oauth) = &entry.oauth {
                for endpoint in [&oauth.authorization_endpoint, &oauth.token_endpoint] {
                    let url = url::Url::parse(endpoint).unwrap();
                    assert_eq!(url.scheme(), "https");
                    assert!(
                        url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                    );
                }
                assert!(!oauth.scopes.is_empty());
            }
            if entry.slug == "github" {
                assert_eq!(entry.methods, [Method::ApiKey, Method::OAuthApp]);
            }
        }
    }
}
