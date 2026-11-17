use super::Accounts;
use crate::infrastructure::upstream::{Http, Replies};
use futures_util::{StreamExt, stream::BoxStream};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::streamable_http_client::{
        SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use secrecy::{ExposeSecret, SecretString};
use sse_stream::Sse;
use std::{collections::HashMap, io, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Client {
    inner: Http,
    accounts: Accounts,
    source: Uuid,
    required: bool,
    link: String,
}
impl Client {
    pub fn new(
        accounts: Accounts,
        source: Uuid,
        required: bool,
        link: String,
        replies: Replies,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            inner: Http::new(replies)?,
            accounts,
            source,
            required,
            link,
        })
    }
    async fn credential(
        &self,
        uri: &str,
    ) -> Result<Option<SecretString>, StreamableHttpError<io::Error>> {
        let credential = self.accounts.bearer(self.source, uri).await.map_err(|_| {
            StreamableHttpError::Client(io::Error::other(format!(
                "linked credential unavailable; link it at {}",
                self.link
            )))
        })?;
        if self.required && credential.is_none() {
            return Err(StreamableHttpError::Client(io::Error::other(format!(
                "account is not linked; link it at {}",
                self.link
            ))));
        }
        Ok(credential)
    }
}
impl StreamableHttpClient for Client {
    type Error = io::Error;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session: Option<Arc<str>>,
        _auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<io::Error>> {
        let mut lease = self
            .accounts
            .source_access(self.source)
            .await
            .map_err(access_error)?;
        let result = tokio::select! { biased;
            error=lease.stopped()=>Err(access_error(error)),
            result=async {
                let credential = self.credential(&uri).await?;
                self.inner
            .post_message(
                uri,
                message,
                session,
                credential.as_ref().map(|v| v.expose_secret().to_owned()),
                headers,
            )
            .await
            }=>result,
        };
        match result {
            Ok(StreamableHttpPostResponse::Sse(stream, session)) => Ok(
                StreamableHttpPostResponse::Sse(guard_stream(stream, lease), session),
            ),
            other => {
                lease.close().await;
                other
            }
        }
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        _auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<io::Error>> {
        let credential = self
            .accounts
            .cleanup_bearer(self.source, &uri)
            .await
            .map_err(access_error)?;
        self.inner
            .delete_session(
                uri,
                session,
                credential.as_ref().map(|v| v.expose_secret().to_owned()),
                headers,
            )
            .await
    }
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session: Option<Arc<str>>,
        last_event: Option<String>,
        _auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<io::Error>> {
        let mut lease = self
            .accounts
            .source_access(self.source)
            .await
            .map_err(access_error)?;
        let result = tokio::select! { biased;
            error=lease.stopped()=>Err(access_error(error)),
            result=async {
                let credential = self.credential(&uri).await?;
                self.inner
            .get_stream(
                uri,
                session,
                last_event,
                credential.as_ref().map(|v| v.expose_secret().to_owned()),
                headers,
            )
            .await
            }=>result,
        };
        match result {
            Ok(stream) => Ok(guard_stream(stream, lease)),
            Err(error) => {
                lease.close().await;
                Err(error)
            }
        }
    }
}

fn access_error(error: impl std::fmt::Display) -> StreamableHttpError<io::Error> {
    StreamableHttpError::Client(io::Error::new(
        io::ErrorKind::PermissionDenied,
        error.to_string(),
    ))
}
fn guard_stream(
    stream: BoxStream<'static, Result<Sse, SseError>>,
    lease: crate::components::access::Watch,
) -> BoxStream<'static, Result<Sse, SseError>> {
    futures_util::stream::unfold(Some((stream, lease)), |state| async move {
        let (mut stream, mut lease) = state?;
        tokio::select! { biased;
            error=lease.stopped()=>{
                lease.close().await;
                Some((Err(SseError::Body(Box::new(io::Error::new(io::ErrorKind::PermissionDenied, error)))), None))
            },
            next=stream.next()=>match next {
                Some(value)=>Some((value, Some((stream, lease)))),
                None=>{ lease.close().await; None },
            },
        }
    }).boxed()
}
