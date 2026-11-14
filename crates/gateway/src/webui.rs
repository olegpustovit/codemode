use axum::{
    Json,
    body::Body,
    extract::Request,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};

struct Asset {
    path: &'static str,
    bytes: &'static [u8],
}
include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

pub fn handler(hostname: String) -> impl Fn(Request) -> std::future::Ready<Response> + Clone {
    move |request| std::future::ready(serve(&hostname, request, ASSETS))
}

fn serve(hostname: &str, request: Request, assets: &[Asset]) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .filter(|_| request.headers().get_all(header::HOST).iter().count() == 1)
        .and_then(|value| value.to_str().ok())
        .and_then(codemode_core::components::workspaces::authority);
    if host.as_deref() != Some(hostname) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = request.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({"error":"route not found"})),
        )
            .into_response();
    }
    if [
        "/mcp",
        "/mcp-admin",
        "/oauth",
        "/.well-known",
        "/healthz",
        "/metrics",
    ]
    .iter()
    .any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    let asset = assets
        .binary_search_by_key(&path, |asset| asset.path)
        .ok()
        .map(|index| &assets[index]);
    let (asset, cache) = match asset {
        Some(asset) if path == "/index.html" => (asset, "no-cache"),
        Some(asset) if path.starts_with("/assets/") => {
            (asset, "public, max-age=31536000, immutable")
        }
        Some(asset) => (asset, "public, max-age=3600"),
        None if path == "/assets"
            || path.starts_with("/assets/")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|last| last.contains('.')) =>
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        None => match assets.iter().find(|asset| asset.path == "/index.html") {
            Some(index) => (index, "no-cache"),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Control plane UI has not been built",
                )
                    .into_response();
            }
        },
    };
    let content_type = match asset.path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("webmanifest") => "application/manifest+json",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    let mut response = (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors 'none'; base-uri 'self'; object-src 'none'",
            ),
        ],
        asset.bytes,
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, asset.bytes.len().into());
    if request.method() == Method::HEAD {
        *response.body_mut() = Body::empty();
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[Asset] = &[
        Asset {
            path: "/assets/index-123.css",
            bytes: b"body{}",
        },
        Asset {
            path: "/assets/index-123.js",
            bytes: b"export{}",
        },
        Asset {
            path: "/icon.svg",
            bytes: b"<svg/>",
        },
        Asset {
            path: "/index.html",
            bytes: b"<!doctype html>",
        },
        Asset {
            path: "/site.webmanifest",
            bytes: b"{}",
        },
    ];
    fn request(method: &str, path: &str, host: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap()
    }
    #[tokio::test]
    async fn history_fallback_caching_and_reserved_routes() {
        for path in [
            "/",
            "/index.html",
            "/sources/123/accounts?oauth=linked",
            "/authorize?client_id=fixture",
        ] {
            let response = serve(
                "codemode.microapps.space",
                request("GET", path, "codemode.microapps.space"),
                FIXTURE,
            );
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                "frame-ancestors 'none'; base-uri 'self'; object-src 'none'"
            );
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap(),
                b"<!doctype html>".as_slice()
            );
        }
        for (path, mime, cache) in [
            (
                "/assets/index-123.js",
                "text/javascript; charset=utf-8",
                "public, max-age=31536000, immutable",
            ),
            (
                "/assets/index-123.css",
                "text/css; charset=utf-8",
                "public, max-age=31536000, immutable",
            ),
            (
                "/site.webmanifest",
                "application/manifest+json",
                "public, max-age=3600",
            ),
            ("/icon.svg", "image/svg+xml", "public, max-age=3600"),
        ] {
            let response = serve(
                "codemode.microapps.space",
                request("HEAD", path, "codemode.microapps.space"),
                FIXTURE,
            );
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
            assert_eq!(response.headers()[header::CACHE_CONTROL], cache);
            assert_eq!(
                response.headers()[header::CONTENT_LENGTH],
                FIXTURE
                    .iter()
                    .find(|asset| asset.path == path)
                    .unwrap()
                    .bytes
                    .len()
                    .to_string()
            );
            assert!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        for path in [
            "/assets/missing.js",
            "/assets",
            "/unknown.svg",
            "/mcp",
            "/mcp/foo",
            "/mcp-admin/foo",
            "/oauth/missing",
            "/.well-known/missing",
            "/healthz",
            "/metrics/foo",
        ] {
            assert_eq!(
                serve(
                    "codemode.microapps.space",
                    request("GET", path, "codemode.microapps.space"),
                    FIXTURE
                )
                .status(),
                StatusCode::NOT_FOUND
            );
        }
        let response = serve(
            "codemode.microapps.space",
            request("GET", "/api/nope", "codemode.microapps.space"),
            FIXTURE,
        );
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            serve(
                "codemode.microapps.space",
                request("GET", "/sources", "work.microapps.space"),
                FIXTURE
            )
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve(
                "codemode.microapps.space",
                request("POST", "/sources", "codemode.microapps.space"),
                FIXTURE
            )
            .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let mut duplicate = request("GET", "/sources", "codemode.microapps.space");
        duplicate
            .headers_mut()
            .append(header::HOST, "codemode.microapps.space".parse().unwrap());
        assert_eq!(
            serve("codemode.microapps.space", duplicate, FIXTURE).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve(
                "codemode.microapps.space",
                request("GET", "/", "codemode.microapps.space"),
                &[]
            )
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
