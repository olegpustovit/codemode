use super::*;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};

#[derive(Deserialize)]
struct Document {
    client_id: String,
    token_endpoint_auth_method: Option<AuthMethod>,
    #[serde(default, deserialize_with = "present")]
    client_secret: bool,
    #[serde(default, deserialize_with = "present")]
    client_secret_expires_at: bool,
    #[serde(flatten)]
    metadata: ClientMetadata,
}
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok(true)
}

impl OAuth {
    pub fn prepare_client<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        // Keep cold discovery's network stack out of HTTP handler future types.
        Box::pin(async move {
            if id.starts_with("cm_client_") {
                self.client(id).await?;
                return Ok(());
            }
            let url = client_url(id)?;
            let fresh=sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM oauth_clients WHERE client_id=$1 AND updated_at>now()-interval '5 minutes') AS \"fresh!\"",id)
            .fetch_one(&self.db.0).await.map_err(DbError::from)?;
            if fresh {
                return Ok(());
            }
            self.allow_registration()?;
            let _permit = self
                .metadata_requests
                .try_acquire()
                .map_err(|_| Error::Busy)?;
            let host = url.host_str().ok_or(Error::Client)?;
            let port = url.port_or_known_default().ok_or(Error::Client)?;
            let addresses: Vec<SocketAddr> = match url.host() {
                Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
                Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
                Some(url::Host::Domain(_)) => tokio::time::timeout(
                    Duration::from_secs(5),
                    tokio::net::lookup_host((host, port)),
                )
                .await
                .map_err(|_| Error::Client)?
                .map_err(|_| Error::Client)?
                .take(33)
                .collect(),
                None => return Err(Error::Client),
            };
            if addresses.is_empty()
                || addresses.len() > 32
                || addresses.iter().any(|a| !public_address(a.ip()))
            {
                return Err(Error::Client);
            }
            // Pin the checked DNS answer; redirects and proxy environment variables cannot bypass it.
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| Error::Client)?;
            let mut response = client
                .get(url)
                .header("accept", "application/json")
                .send()
                .await
                .map_err(|_| Error::Client)?;
            let json = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim() == "application/json" || v.trim().ends_with("+json"));
            if response.status() != reqwest::StatusCode::OK
                || !json
                || response.content_length().is_some_and(|n| n > 32768)
            {
                return Err(Error::Client);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| Error::Client)? {
                if chunk.len() > 32768 - bytes.len() {
                    return Err(Error::Client);
                }
                bytes.extend_from_slice(&chunk);
            }
            let metadata = parse_document(&bytes, id)?;
            let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
            sqlx::query!("LOCK TABLE oauth_clients IN SHARE ROW EXCLUSIVE MODE")
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
            let available = sqlx::query_scalar!(
            "SELECT count(*)<1024 OR bool_or(client_id=$1) AS \"available!\" FROM oauth_clients",
            id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::from)?;
            if !available {
                return Err(Error::Busy);
            }
            sqlx::query!("INSERT INTO oauth_clients(client_id,metadata) VALUES($1,$2) ON CONFLICT(client_id) DO UPDATE SET metadata=EXCLUDED.metadata,updated_at=now()",id,sqlx::types::Json(&metadata) as _)
            .execute(&mut *tx).await.map_err(DbError::from)?;
            tx.commit().await.map_err(DbError::from)?;
            Ok(())
        })
    }
}

fn parse_document(bytes: &[u8], id: &str) -> Result<ClientMetadata, Error> {
    let mut document: Document = serde_json::from_slice(bytes).map_err(|_| Error::Client)?;
    if document.client_id != id
        || document.client_secret
        || document.client_secret_expires_at
        || document
            .token_endpoint_auth_method
            .is_some_and(|method| method != AuthMethod::None)
    {
        return Err(Error::Client);
    }
    document.metadata.token_endpoint_auth_method = AuthMethod::None;
    document.metadata.validate()?;
    Ok(document.metadata)
}

fn client_url(value: &str) -> Result<url::Url, Error> {
    if value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(Error::Client);
    }
    let url = url::Url::parse(value).map_err(|_| Error::Client)?;
    let raw_path = value
        .split_once("://")
        .map(|(_, authority)| {
            authority.split_once('/').map_or("", |(_, path)| {
                path.split(['?', '#']).next().unwrap_or_default()
            })
        })
        .ok_or(Error::Client)?;
    let dots = raw_path.split('/').any(|segment| {
        matches!(
            segment.to_ascii_lowercase().as_str(),
            "." | ".." | "%2e" | ".%2e" | "%2e." | "%2e%2e"
        )
    });
    if url.scheme() != "https"
        || url.path() == "/"
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
        || dots
    {
        return Err(Error::Client);
    }
    Ok(url)
}
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (18..=19).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_rejects_private_addresses_ambiguous_urls_and_shared_secrets() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.0.8",
            "192.0.2.1",
            "0.0.0.0",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::1",
            "3fff::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(address.parse().unwrap()));
        }
        assert!(client_url("https://client.example/oauth.json").is_ok());
        for value in [
            "http://client.example/oauth.json",
            "https://client.example/",
            "https://user@client.example/oauth.json",
            "https://client.example/oauth.json#fragment",
            "https://client.example/a/../oauth.json",
            "https://client.example/%2e%2e/oauth.json",
            "https://client.example/\\oauth.json",
        ] {
            assert!(client_url(value).is_err());
        }
        for field in ["client_secret", "client_secret_expires_at"] {
            let document = serde_json::json!({"client_id":"https://client.example/oauth.json","redirect_uris":["https://client.example/callback"],field:null});
            assert!(
                parse_document(
                    &serde_json::to_vec(&document).unwrap(),
                    "https://client.example/oauth.json"
                )
                .is_err()
            );
        }
        let mut document = serde_json::json!({"client_id":"https://client.example/oauth.json","redirect_uris":["https://client.example/callback"]});
        let bytes = serde_json::to_vec(&document).unwrap();
        assert_eq!(
            parse_document(&bytes, "https://client.example/oauth.json")
                .unwrap()
                .token_endpoint_auth_method,
            AuthMethod::None
        );
        assert!(parse_document(&bytes, "https://other.example/client.json").is_err());
        document["token_endpoint_auth_method"] = serde_json::json!("client_secret_basic");
        assert!(
            parse_document(
                &serde_json::to_vec(&document).unwrap(),
                "https://client.example/oauth.json"
            )
            .is_err()
        );
    }
}
