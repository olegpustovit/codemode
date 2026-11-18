pub use super::providers::{Entry, Method, registry};
use super::sources::{Record, Source, SourceInput, Sources, Transport};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    #[serde(flatten)]
    pub entry: Entry,
    pub source_id: Option<Uuid>,
    pub connections: Vec<Connection>,
    pub state: String,
    pub tool_count: i64,
    pub tool_names: Vec<String>,
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub source_id: Uuid,
    pub slug: String,
    pub name: String,
    pub state: String,
    pub tool_count: i64,
    pub error: Option<String>,
}

impl From<&Record> for Connection {
    fn from(source: &Record) -> Self {
        Self {
            source_id: source.id,
            slug: source.source.slug.clone(),
            name: source.name.clone(),
            state: source.status.clone(),
            tool_count: source.tool_count,
            error: source.last_error.clone(),
        }
    }
}

fn matches_entry(source: &Record, entry: &Entry) -> bool {
    matches!(&source.source.transport, Transport::McpHttp { url, .. } if url.trim_end_matches('/') == entry.url.trim_end_matches('/'))
}

fn valid_namespace(namespace: &str) -> bool {
    let mut bytes = namespace.bytes();
    namespace.len() <= 31
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "namespace must start with a lowercase letter and contain at most 31 lowercase letters, numbers, or underscores"
    )]
    Namespace,
    #[error("That namespace is taken")]
    NamespaceTaken,
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
        let mut connections: Vec<_> = sources
            .iter()
            .filter(|source| matches_entry(source, &entry))
            .collect();
        connections.sort_by_key(|source| source.source.slug != entry.slug);
        let source = connections.first().copied();
        let mut tool_names = Vec::new();
        for source in &connections {
            matched.insert(source.id);
            if source.status == "ready" {
                tool_names.extend(managed.tool_names(source.id).await);
            }
        }
        result.push(Integration {
            source_id: source.map(|s| s.id),
            connections: connections
                .iter()
                .map(|source| Connection::from(*source))
                .collect(),
            state: source.map_or("available", |s| s.status.as_str()).into(),
            tool_count: connections.iter().map(|s| s.tool_count).sum(),
            tool_names,
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
            connections: vec![Connection::from(&source)],
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

pub async fn add(sources: &Sources, slug: &str, namespace: Option<&str>) -> Result<Uuid, Error> {
    let entry = registry()?
        .into_iter()
        .find(|entry| entry.slug == slug)
        .ok_or(Error::NotFound)?;
    let (slug, name) = if let Some(namespace) = namespace {
        if !valid_namespace(namespace) {
            return Err(Error::Namespace);
        }
        (
            format!("{}_{namespace}", entry.slug),
            Some(format!("{} ({namespace})", entry.name)),
        )
    } else {
        if let Some(existing) = sources
            .list()
            .await?
            .into_iter()
            .find(|record| matches_entry(record, &entry))
        {
            return Ok(existing.id);
        }
        (entry.slug, None)
    };
    sources
        .save_input(
            None,
            SourceInput {
                source: Source {
                    slug,
                    requires_account: !entry.methods.contains(&Method::Open),
                    transport: Transport::McpHttp {
                        url: entry.url,
                        bearer_env: None,
                    },
                },
                name,
                icon: None,
            },
        )
        .await
        .map_err(|error| match error {
            super::sources::Error::Conflict if namespace.is_some() => Error::NamespaceTaken,
            other => Error::Source(other),
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_is_a_bounded_python_identifier_suffix() {
        assert!(valid_namespace("personal_2"));
        assert!(valid_namespace(&"a".repeat(31)));
        for invalid in ["", "_personal", "2nd", "Personal", "a-b", &"a".repeat(32)] {
            assert!(!valid_namespace(invalid));
        }
    }
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
