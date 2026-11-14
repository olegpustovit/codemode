use futures_util::{StreamExt, stream::BoxStream};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::{
        CallToolResult, ClientJsonRpcMessage, ClientRequest, JsonRpcMessage, JsonRpcResponse,
        JsonRpcVersion2_0, RequestId, ServerJsonRpcMessage, ServerResult,
    },
    service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport,
        streamable_http_client::{
            SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
        },
    },
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::value::RawValue;
use sse_stream::{Sse, SseStream};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MAX_REPLY: usize = 1024 * 1024;

#[derive(Default, Clone)]
pub struct Replies(Arc<Mutex<BTreeMap<uuid::Uuid, (Instant, Reply)>>>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    #[serde(skip)]
    credential: Option<SecretString>,
    #[serde(default)]
    pub is_error: bool,
    pub content: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    pub structured_content: Option<Box<RawValue>>,
}

fn present_raw<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    id: Option<&'a RawValue>,
    #[serde(borrow)]
    result: Option<&'a RawValue>,
}

impl Replies {
    pub fn take(&self, result: rmcp::model::CallToolResult) -> Result<Reply, String> {
        let id = result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("codemodeReply"))
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or_else(|| "invalid upstream response envelope".to_owned())?;
        self.0
            .lock()
            .map_err(|_| "upstream response store unavailable".to_owned())?
            .remove(&id)
            .map(|(_, v)| v)
            .ok_or_else(|| "upstream response expired".into())
    }

    fn intercept(&self, bytes: &[u8]) -> Result<ServerJsonRpcMessage, io::Error> {
        self.intercept_with_credential(bytes, None)
    }

    fn intercept_with_credential(
        &self,
        bytes: &[u8],
        credential: Option<&SecretString>,
    ) -> Result<ServerJsonRpcMessage, io::Error> {
        if bytes.len() > MAX_REPLY {
            return Err(io::Error::other("upstream response exceeds 1 MB"));
        }
        let envelope: Envelope<'_> = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        if let (Some(id), Some(result)) = (envelope.id, envelope.result) {
            if let Ok(mut reply) = serde_json::from_str::<Reply>(result.get()) {
                if reply.is_error {
                    reply.credential = credential.cloned();
                }
                let key = uuid::Uuid::new_v4();
                let id: RequestId = serde_json::from_str(id.get()).map_err(io::Error::other)?;
                let mut result = CallToolResult::success(vec![]);
                result.structured_content =
                    Some(serde_json::json!({"codemodeReply": key.to_string()}));
                let response = JsonRpcMessage::Response(JsonRpcResponse {
                    jsonrpc: JsonRpcVersion2_0,
                    id,
                    result: ServerResult::CallToolResult(result),
                });
                let mut replies = self
                    .0
                    .lock()
                    .map_err(|_| io::Error::other("upstream response store unavailable"))?;
                replies.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
                if replies.len() >= 128 {
                    return Err(io::Error::other("upstream response store is full"));
                }
                replies.insert(key, (Instant::now(), reply));
                return Ok(response);
            }
        }
        serde_json::from_slice(bytes).map_err(io::Error::other)
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    pub replies: Replies,
}

impl Http {
    pub fn new(replies: Replies) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .build()?,
            replies,
        })
    }
}

impl StreamableHttpClient for Http {
    type Error = io::Error;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<io::Error>> {
        let is_call = matches!(&message,ClientJsonRpcMessage::Request(request) if matches!(&request.request,ClientRequest::CallToolRequest(_)));
        if !is_call {
            return self
                .client
                .post_message_with_max_sse_event_size(
                    uri, message, session_id, auth, headers, MAX_REPLY,
                )
                .await
                .map_err(map_http_error);
        }
        let credential = auth.as_ref().map(|value| SecretString::from(value.clone()));
        let mut request = self
            .client
            .post(uri.as_ref())
            .header("accept", "application/json, text/event-stream")
            .json(&message);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(session) = session_id {
            request = request.header("mcp-session-id", session.as_ref());
        }
        if let Some(auth) = auth {
            request = request.bearer_auth(auth);
        }
        let response = request
            .send()
            .await
            .map_err(|e| StreamableHttpError::Client(io::Error::other(e.without_url())))?;
        if !response.status().is_success() {
            return Err(StreamableHttpError::Client(io::Error::other(format!(
                "upstream HTTP {}",
                response.status().as_u16()
            ))));
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let replies = self.replies.clone();
        if sse {
            let bounded = response.bytes_stream().scan(0usize, |total, chunk| {
                let chunk = chunk
                    .map_err(|e| io::Error::other(e.without_url()))
                    .and_then(|c| {
                        *total = total.saturating_add(c.len());
                        if *total > MAX_REPLY {
                            Err(io::Error::other("upstream response exceeds 1 MB"))
                        } else {
                            Ok(c)
                        }
                    });
                std::future::ready(Some(chunk))
            });
            let events = SseStream::from_bytes_stream(bounded)
                .map(move |event| {
                    let mut event = event?;
                    if let Some(data) = event.data.as_mut().filter(|d| !d.trim().is_empty()) {
                        let message = replies
                            .intercept_with_credential(data.as_bytes(), credential.as_ref())
                            .map_err(|e| SseError::Body(Box::new(e)))?;
                        *data = serde_json::to_string(&message)
                            .map_err(|e| SseError::Body(Box::new(e)))?;
                    }
                    Ok(event)
                })
                .boxed();
            return Ok(StreamableHttpPostResponse::Sse(events, session));
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| StreamableHttpError::Client(io::Error::other(e.without_url())))?
        {
            if bytes.len() + chunk.len() > MAX_REPLY {
                return Err(StreamableHttpError::Client(io::Error::other(
                    "upstream response exceeds 1 MB",
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        let message = replies
            .intercept_with_credential(&bytes, credential.as_ref())
            .map_err(StreamableHttpError::Client)?;
        Ok(StreamableHttpPostResponse::Json(message, session))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<io::Error>> {
        self.client
            .delete_session(uri, session, auth, headers)
            .await
            .map_err(map_http_error)
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session: Option<Arc<str>>,
        last_event: Option<String>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<io::Error>> {
        let replies = self.replies.clone();
        let credential = auth.as_ref().map(|value| SecretString::from(value.clone()));
        let stream = self
            .client
            .get_stream_with_max_sse_event_size(uri, session, last_event, auth, headers, MAX_REPLY)
            .await
            .map_err(map_http_error)?;
        Ok(stream
            .map(move |event| {
                let mut event = event?;
                if let Some(data) = event.data.as_mut().filter(|d| !d.trim().is_empty()) {
                    let message = replies
                        .intercept_with_credential(data.as_bytes(), credential.as_ref())
                        .map_err(|e| SseError::Body(Box::new(e)))?;
                    *data =
                        serde_json::to_string(&message).map_err(|e| SseError::Body(Box::new(e)))?;
                }
                Ok(event)
            })
            .boxed())
    }
}

fn map_http_error(error: StreamableHttpError<reqwest::Error>) -> StreamableHttpError<io::Error> {
    // Authentication headers and response bodies stay out of diagnostic messages.
    let message = match error {
        StreamableHttpError::ServerDoesNotSupportSse => {
            return StreamableHttpError::ServerDoesNotSupportSse;
        }
        StreamableHttpError::SessionExpired => return StreamableHttpError::SessionExpired,
        _ => "upstream HTTP transport failed",
    };
    StreamableHttpError::Client(io::Error::other(message))
}

pub struct Stdio {
    child: tokio::process::Child,
    read: BufReader<tokio::process::ChildStdout>,
    write: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    line: Vec<u8>,
    replies: Replies,
}

impl Stdio {
    pub fn new(mut command: tokio::process::Command, replies: Replies) -> io::Result<Self> {
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let read = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("child stdout missing"))?,
        );
        let write = Arc::new(tokio::sync::Mutex::new(
            child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("child stdin missing"))?,
        ));
        Ok(Self {
            child,
            read,
            write,
            line: Vec::new(),
            replies,
        })
    }
}

impl Transport<RoleClient> for Stdio {
    type Error = io::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let write = self.write.clone();
        async move {
            let mut bytes = serde_json::to_vec(&item).map_err(io::Error::other)?;
            bytes.push(b'\n');
            let mut write = write.lock().await;
            write.write_all(&bytes).await?;
            write.flush().await
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            let chunk = self.read.fill_buf().await.ok()?;
            if chunk.is_empty() {
                return None;
            }
            let n = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(chunk.len());
            if self.line.len() + n > MAX_REPLY {
                return None;
            }
            let done = chunk[n - 1] == b'\n';
            self.line.extend_from_slice(&chunk[..n]);
            self.read.consume(n);
            if done {
                let result = self.replies.intercept(&self.line).ok();
                self.line.clear();
                return result;
            }
        }
    }
    async fn close(&mut self) -> io::Result<()> {
        let _ = self.child.start_kill();
        self.child.wait().await?;
        Ok(())
    }
}

impl Reply {
    pub fn result(self) -> Result<Box<RawValue>, String> {
        #[derive(Deserialize)]
        struct Text {
            text: Option<String>,
        }
        if self.is_error {
            let text: Vec<Text> = serde_json::from_str(self.content.get())
                .map_err(|_| "invalid upstream error content".to_owned())?;
            let mut message = text
                .into_iter()
                .filter_map(|t| t.text)
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(credential) = self.credential.filter(|s| !s.expose_secret().is_empty()) {
                message = message.replace(credential.expose_secret(), "[REDACTED]");
            }
            return Err(message);
        }
        if let Some(value) = self.structured_content {
            return Ok(value);
        }
        let content: Vec<&RawValue> = serde_json::from_str(self.content.get())
            .map_err(|_| "invalid upstream content".to_owned())?;
        if content.len() == 1 {
            if let Ok(Text { text: Some(text) }) = serde_json::from_str::<Text>(content[0].get()) {
                return Ok(RawValue::from_string(text.clone()).unwrap_or(
                    serde_json::value::to_raw_value(&text)
                        .map_err(|_| "invalid upstream text".to_owned())?,
                ));
            }
        }
        Ok(self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_results_pass_through_with_constant_allocation_budget() {
        let bytes = format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"content":[],"structuredContent":{{"items":[{}]}}}}}}"#,
            vec![r#"{"id":1,"value":"text"}"#; 5000].join(",")
        );
        let replies = Replies::default();
        let mut output = None;
        let allocations = allocation_counter::measure(|| {
            let message = replies.intercept(bytes.as_bytes()).unwrap();
            let rmcp::model::JsonRpcMessage::Response(response) = message else {
                panic!("response expected");
            };
            let rmcp::model::ServerResult::CallToolResult(result) = response.result else {
                panic!("tool result expected");
            };
            output = Some(replies.take(result).unwrap().result().unwrap());
        });
        assert!(
            allocations.count_total <= 40,
            "{} allocations exceed the 40-allocation pass-through budget",
            allocations.count_total
        );
        assert!(output.unwrap().get().starts_with("{\"items\":["));
    }
    #[test]
    fn distinguishes_json_text_plain_text_and_tool_errors() {
        for (input, expected) in [
            (r#"{"content":[],"structuredContent":null}"#, "null"),
            (
                r#"{"content":[{"type":"text","text":"{\"ok\":true}"}]}"#,
                r#"{"ok":true}"#,
            ),
            (
                r#"{"content":[{"type":"text","text":"hello"}]}"#,
                r#""hello""#,
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<Reply>(input)
                    .unwrap()
                    .result()
                    .unwrap()
                    .get(),
                expected
            );
        }
        assert_eq!(
            serde_json::from_str::<Reply>(
                r#"{"isError":true,"content":[{"type":"text","text":"precise failure"}]}"#
            )
            .unwrap()
            .result()
            .unwrap_err(),
            "precise failure"
        );
    }
}
