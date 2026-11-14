use super::Accounts;
use crate::infrastructure::upstream::{Http, Replies};
use futures_util::stream::BoxStream;
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
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        _auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<io::Error>> {
        let credential = self.credential(&uri).await?;
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
    }
}
