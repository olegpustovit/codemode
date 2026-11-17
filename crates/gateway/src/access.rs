use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use codemode_core::components::access::{Access, Error};
use futures_util::StreamExt;
use std::future::Future;
use uuid::Uuid;

pub fn response(error: Error) -> Response {
    let status = match error {
        Error::Denied => StatusCode::FORBIDDEN,
        Error::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        [("cache-control", "no-store")],
        axum::Json(serde_json::json!({"error":error.to_string()})),
    )
        .into_response()
}

/// Keep the lease through pending handlers and body streaming. The same watch
/// covers tenant initialization, which can start source connections.
pub async fn request(
    access: &Access,
    tenant: Uuid,
    future: impl Future<Output = Response>,
) -> Response {
    if !access.enforced() {
        return future.await;
    }
    let mut lease = match access.watch(tenant).await {
        Ok(lease) => lease,
        Err(error) => return response(error),
    };
    let result = tokio::select! { biased;
        error=lease.stopped()=>Err(error),
        response=future=>Ok(response),
    };
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            lease.close().await;
            return self::response(error);
        }
    };
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold(
        Some((body.into_data_stream(), lease)),
        |state| async move {
            let (mut body, mut lease) = state?;
            tokio::select! { biased;
                error=lease.stopped()=>{
                    lease.close().await;
                    Some((Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, error)), None))
                },
                next=body.next()=> match next {
                    Some(value)=>Some((value.map_err(std::io::Error::other), Some((body, lease)))),
                    None=>{ lease.close().await; None },
                },
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}
