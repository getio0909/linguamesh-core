#![doc = "Anthropic Messages 提供商适配器。"]

use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use linguamesh_domain::{
    ChunkingError, DEFAULT_TRANSLATION_CHUNK_BYTES, EndpointConfiguration, ErrorKind,
    ModelDescriptor, ModelSource, ProtectedSource, ProtectedTextError, SecretValue,
    TranslationError, TranslationRequest, UsageRecord, protect_source_text_with_glossary,
};
use linguamesh_provider_api::{
    ModelProvider, TranslationStream, TranslationStreamEvent, retry_after_ms, translation_prompt,
};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// 配置 Anthropic Messages 端点。
pub struct AnthropicConfig {
    /// 通常以 `/v1/` 结尾的基础地址。
    pub base_url: String,
    /// 可选的一次性内存凭据。
    pub credential: Option<SecretValue>,
    /// 用户手动输入的模型标识；Anthropic 不提供通用模型列表端点。
    pub model_id: Option<String>,
    /// 可选的不含凭据代理地址。
    pub proxy_url: Option<String>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
}

impl AnthropicConfig {
    /// 创建没有凭据的手动模型配置。
    #[must_use]
    pub fn without_credential(base_url: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credential: None,
            model_id: Some(model_id.into()),
            proxy_url: None,
            request_timeout: Duration::from_secs(30),
        }
    }

    /// 创建携带一次性内存凭据的手动模型配置。
    #[must_use]
    pub fn with_credential(
        base_url: impl Into<String>,
        model_id: impl Into<String>,
        credential: SecretValue,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            credential: Some(credential),
            model_id: Some(model_id.into()),
            proxy_url: None,
            request_timeout: Duration::from_secs(30),
        }
    }

    /// 设置不含凭据的代理地址。
    #[must_use]
    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }
}

impl fmt::Debug for AnthropicConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnthropicConfig")
            .field("base_url", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("model_id", &self.model_id)
            .field("has_proxy_url", &self.proxy_url.is_some())
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// 实现 Anthropic 模型手动选择和 Messages 流式聊天。
#[derive(Clone)]
pub struct AnthropicProvider {
    client: Client,
    base_url: Url,
    model_id: Option<String>,
    credential: Arc<Mutex<CredentialState>>,
    session_cancellation: CancellationToken,
}

enum CredentialState {
    NotRequired,
    Available(SecretValue),
    Cleared,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let credential_state =
            self.credential
                .lock()
                .map_or("poisoned", |credential| match &*credential {
                    CredentialState::NotRequired => "not_required",
                    CredentialState::Available(_) => "available_redacted",
                    CredentialState::Cleared => "cleared",
                });
        formatter
            .debug_struct("AnthropicProvider")
            .field("base_url", &"[REDACTED]")
            .field("model_id", &self.model_id)
            .field("credential_state", &credential_state)
            .field("session_closed", &self.session_cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl AnthropicProvider {
    /// 在请求宿主秘密之前验证端点策略。
    pub fn validate_endpoint(base_url: &str) -> Result<(), TranslationError> {
        validated_base_url(base_url).map(|_| ())
    }

    /// 创建拒绝跨源重定向的适配器。
    pub fn new(config: AnthropicConfig) -> Result<Self, TranslationError> {
        let base_url = validated_base_url(&config.base_url)?;
        let mut client_builder = Client::builder()
            .redirect(Policy::none())
            .timeout(config.request_timeout);
        if let Some(proxy_url) = config.proxy_url.as_deref() {
            let proxy =
                reqwest::Proxy::all(proxy_url).map_err(|error| map_reqwest_error(&error))?;
            client_builder = client_builder.proxy(proxy);
        } else if base_url.scheme() == "http" {
            client_builder = client_builder.no_proxy();
        }
        let client = client_builder
            .build()
            .map_err(|error| map_reqwest_error(&error))?;
        Ok(Self {
            client,
            base_url,
            model_id: config.model_id,
            credential: Arc::new(Mutex::new(
                config
                    .credential
                    .map_or(CredentialState::NotRequired, CredentialState::Available),
            )),
            session_cancellation: CancellationToken::new(),
        })
    }

    /// 取消会话并清除共享引用中的凭据。
    pub fn close_session(&self) {
        self.session_cancellation.cancel();
        match self.credential.lock() {
            Ok(mut credential) => {
                if matches!(&*credential, CredentialState::Available(_)) {
                    *credential = CredentialState::Cleared;
                }
            }
            Err(poisoned) => {
                *poisoned.into_inner() = CredentialState::Cleared;
            }
        }
    }

    fn request(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, TranslationError> {
        if self.session_cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
        let mut request = request.header("anthropic-version", ANTHROPIC_VERSION);
        let mut credential = self.credential.lock().map_err(|poisoned| {
            *poisoned.into_inner() = CredentialState::Cleared;
            TranslationError::new(
                ErrorKind::Internal,
                "The provider credential session is unavailable.",
            )
        })?;
        if self.session_cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
        match &mut *credential {
            CredentialState::NotRequired => Ok(request),
            CredentialState::Available(secret) => {
                request = request.header("x-api-key", secret.expose_secret());
                Ok(request)
            }
            CredentialState::Cleared => Err(TranslationError::new(
                ErrorKind::SecretUnavailable,
                "The provider credential session was cleared.",
            )),
        }
    }

    fn endpoint(&self, path: &str) -> Result<Url, TranslationError> {
        self.base_url.join(path).map_err(|_| {
            TranslationError::new(ErrorKind::InvalidEndpoint, "Provider endpoint is invalid.")
        })
    }

    async fn translate_protected_stream(
        &self,
        request: TranslationRequest,
        protected: ProtectedSource,
        cancellation: CancellationToken,
    ) -> Result<TranslationStream, TranslationError> {
        let marker_instruction = if protected.is_empty() {
            String::new()
        } else {
            " Preserve every opaque marker such as __LINGUAMESH_PROTECTED_0__ exactly once."
                .to_owned()
        };
        let mut span_restorer = protected.restorer();
        let body = MessagesRequest {
            model: request.model_id,
            max_tokens: DEFAULT_MAX_TOKENS,
            stream: true,
            system: translation_prompt(
                &request.target_locale,
                request.quality_mode,
                Some(&request.preset),
                &marker_instruction,
            ),
            messages: vec![Message {
                role: "user",
                content: format!("<source>\n{}\n</source>", request.source_text),
            }],
        };
        let request = self
            .request(self.client.post(self.endpoint("messages")?))?
            .json(&body)
            .send();
        let session_cancellation = self.session_cancellation.clone();
        let response = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TranslationError::cancelled()),
            () = session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            response = request => response.map_err(|error| map_reqwest_error(&error))?,
        };
        let response = ensure_success(response)?;
        let mut bytes = response.bytes_stream();
        let stream = try_stream! {
            let mut decoder = SseDecoder::default();
            let mut total_bytes = 0usize;
            let mut completed = false;
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => Err(TranslationError::cancelled()),
                    () = session_cancellation.cancelled() => Err(TranslationError::cancelled()),
                    item = bytes.next() => Ok(item),
                }?;
                let Some(chunk) = next else { break; };
                let chunk = chunk.map_err(|error| map_reqwest_error(&error))?;
                total_bytes = total_bytes.saturating_add(chunk.len());
                if total_bytes > MAX_RESPONSE_BYTES {
                    Err(TranslationError::new(
                        ErrorKind::MalformedResponse,
                        "Provider stream exceeded the response-size limit.",
                    ))?;
                }
                for message in decoder.push(&chunk)? {
                    match message {
                        SseMessage::Delta(text) if !text.is_empty() => {
                            let safe_delta = span_restorer
                                .push(&text)
                                .map_err(|error| map_protected_error(&error))?;
                            if !safe_delta.is_empty() {
                                yield TranslationStreamEvent::Text(safe_delta);
                            }
                        }
                        SseMessage::Delta(_) => {}
                        SseMessage::Usage(usage) => {
                            yield TranslationStreamEvent::Usage(usage);
                        }
                        SseMessage::Done => { completed = true; break; }
                    }
                }
                if completed { break; }
            }
            if !completed {
                Err(TranslationError::new(
                    ErrorKind::MalformedResponse,
                    "Provider stream ended before the completion marker.",
                ))?;
            }
            let safe_tail = span_restorer
                .finish()
                .map_err(|error| map_protected_error(&error))?;
            if !safe_tail.is_empty() {
                yield TranslationStreamEvent::Text(safe_tail);
            }
        };
        Ok(Box::pin(stream))
    }
}

fn validated_base_url(base_url: &str) -> Result<Url, TranslationError> {
    let endpoint = EndpointConfiguration::parse(base_url).map_err(|_| {
        TranslationError::new(
            ErrorKind::InvalidEndpoint,
            "Provider endpoint is invalid or unsafe.",
        )
    })?;
    Url::parse(endpoint.as_str()).map_err(|_| {
        TranslationError::new(
            ErrorKind::InvalidEndpoint,
            "Provider endpoint is invalid or unsafe.",
        )
    })
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn list_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError> {
        Ok(self
            .model_id
            .as_ref()
            .map(|id| {
                vec![ModelDescriptor {
                    id: id.clone(),
                    display_name: id.clone(),
                    source: ModelSource::Manual,
                }]
            })
            .unwrap_or_default())
    }

    async fn translate_stream(
        &self,
        request: TranslationRequest,
        cancellation: CancellationToken,
    ) -> Result<TranslationStream, TranslationError> {
        let protected = protect_source_text_with_glossary(
            &request.source_text,
            request.source_locale.as_deref(),
            &request.target_locale,
            request.glossary.as_ref(),
        )
        .map_err(|error| {
            TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
        })?;
        let max_chunk_bytes = request
            .max_chunk_bytes
            .unwrap_or(DEFAULT_TRANSLATION_CHUNK_BYTES);
        let mut chunks = protected
            .chunks(max_chunk_bytes)
            .map_err(map_chunking_error)?;
        let first_chunk = chunks.drain(..1).next().ok_or_else(|| {
            TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Translation source produced no chunks.",
            )
        })?;
        let mut first_request = request.clone();
        first_request.source_text = first_chunk.text().to_owned();
        let first_stream = self
            .translate_protected_stream(first_request, first_chunk, cancellation.clone())
            .await?;
        let provider = self.clone();
        let stream = try_stream! {
            let mut chunk_stream = first_stream;
            while let Some(delta) = chunk_stream.next().await { yield delta?; }
            for chunk in chunks {
                let mut chunk_request = request.clone();
                chunk_request.source_text = chunk.text().to_owned();
                let mut chunk_stream = provider
                    .translate_protected_stream(chunk_request, chunk, cancellation.clone())
                    .await?;
                while let Some(delta) = chunk_stream.next().await { yield delta?; }
            }
        };
        Ok(Box::pin(stream))
    }
}

fn map_chunking_error(error: ChunkingError) -> TranslationError {
    TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
}

fn map_protected_error(error: &ProtectedTextError) -> TranslationError {
    TranslationError::new(ErrorKind::MalformedResponse, error.to_string())
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    stream: bool,
    system: String,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct StreamResponse {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<StreamDelta>,
    #[serde(default)]
    message: Option<MessageStart>,
    #[serde(default)]
    usage: Option<UsageResponse>,
}

#[derive(Deserialize)]
struct MessageStart {
    #[serde(default)]
    usage: Option<UsageResponse>,
}

#[derive(Deserialize)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

enum SseMessage {
    Delta(String),
    Usage(UsageRecord),
    Done,
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)]
struct UsageResponse {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

impl UsageResponse {
    fn into_record(self) -> UsageRecord {
        UsageRecord::provider_reported(self.input_tokens, self.output_tokens, self.total_tokens)
    }
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &Bytes) -> Result<Vec<SseMessage>, TranslationError> {
        self.buffer.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some((position, delimiter_len)) = find_event_boundary(&self.buffer) {
            let event = self.buffer.drain(..position).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            if let Some(message) = parse_event(&event)? {
                output.push(message);
            }
        }
        Ok(output)
    }
}

fn find_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2))
        .or_else(|| {
            buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| (position, 4))
        })
}

fn parse_event(event: &[u8]) -> Result<Option<SseMessage>, TranslationError> {
    let text = std::str::from_utf8(event).map_err(|_| {
        TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider stream contained invalid UTF-8.",
        )
    })?;
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(None);
    }
    let response: StreamResponse = serde_json::from_str(&data).map_err(|_| {
        TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider stream contained malformed JSON.",
        )
    })?;
    if let Some(usage) = response
        .message
        .and_then(|message| message.usage)
        .or(response.usage)
    {
        return Ok(Some(SseMessage::Usage(usage.into_record())));
    }
    if response.event_type == "message_stop" {
        return Ok(Some(SseMessage::Done));
    }
    if response.event_type == "content_block_delta"
        && let Some(delta) = response.delta
        && delta.delta_type == "text_delta"
    {
        return Ok(Some(SseMessage::Delta(delta.text.unwrap_or_default())));
    }
    Ok(None)
}

fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, TranslationError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(retry_after_ms);
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ErrorKind::Authentication,
        StatusCode::NOT_FOUND => ErrorKind::ModelUnavailable,
        _ => ErrorKind::Network,
    };
    Err(TranslationError::new(
        kind,
        format!("Provider request failed with HTTP status {status}."),
    )
    .with_retry_after_ms(retry_after))
}

fn map_reqwest_error(error: &reqwest::Error) -> TranslationError {
    let kind = if error.is_timeout() {
        ErrorKind::Timeout
    } else {
        ErrorKind::Network
    };
    let message = if error.is_timeout() {
        "Provider request timed out."
    } else {
        "Provider network request failed."
    };
    TranslationError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use super::{AnthropicConfig, AnthropicProvider, SseDecoder, SseMessage};
    use bytes::Bytes;
    use futures_util::StreamExt;
    use linguamesh_domain::{ErrorKind, SecretValue, TranslationRequest};
    use linguamesh_provider_api::ModelProvider;
    use std::fmt::Write;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn decoder_handles_fragmented_utf8_and_message_events() {
        let payload = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let bytes = payload.as_bytes();
        let split = payload.find('好').expect("unicode split") + 1;
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(&Bytes::copy_from_slice(&bytes[..split]))
                .expect("first")
                .is_empty()
        );
        let messages = decoder
            .push(&Bytes::copy_from_slice(&bytes[split..]))
            .expect("second");
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], SseMessage::Delta(text) if text == "你好"));
        assert!(matches!(messages[1], SseMessage::Done));
    }

    #[test]
    fn decoder_extracts_message_start_and_delta_usage() {
        let payload = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":8}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut decoder = SseDecoder::default();
        let messages = decoder
            .push(&Bytes::from_static(payload.as_bytes()))
            .expect("usage events");
        assert!(matches!(
            &messages[0],
            SseMessage::Usage(record) if record.input_tokens == Some(8)
        ));
        assert!(matches!(
            &messages[1],
            SseMessage::Usage(record) if record.output_tokens == Some(3)
        ));
        assert!(matches!(messages[2], SseMessage::Done));
    }

    #[test]
    fn diagnostics_redact_endpoint_and_credential() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_ANTHROPIC_DEBUG_SECRET_123456");
        const ENDPOINT: &str = "https://api.anthropic.com/v1/";
        let config = AnthropicConfig::with_credential(
            ENDPOINT,
            "claude-test",
            SecretValue::new(SECRET_CANARY),
        );
        let config_debug = format!("{config:?}");
        assert!(!config_debug.contains(SECRET_CANARY));
        assert!(!config_debug.contains(ENDPOINT));
        let provider = AnthropicProvider::new(config).expect("provider");
        let provider_debug = format!("{provider:?}");
        assert!(!provider_debug.contains(SECRET_CANARY));
        assert!(!provider_debug.contains(ENDPOINT));
    }

    #[tokio::test]
    async fn manual_model_is_listed_and_messages_stream_is_real() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("connection");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let body_start = loop {
                let read = socket.read(&mut chunk).await.expect("request");
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..body_start]);
            assert!(headers.contains("anthropic-version: 2023-06-01"));
            assert!(headers.contains("x-api-key: test-key"));
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("content length");
            while request.len() - body_start < content_length {
                let read = socket.read(&mut chunk).await.expect("request body");
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&request[body_start..body_start + content_length])
                    .expect("request json");
            assert_eq!(body["model"], "claude-test");
            assert_eq!(body["stream"], true);
            let mut events = String::new();
            for text in ["你好", "，Anthropic", "！"] {
                writeln!(
                    &mut events,
                    "event: content_block_delta\ndata: {}\n",
                    serde_json::json!({
                        "type": "content_block_delta",
                        "delta": {"type": "text_delta", "text": text}
                    })
                )
                .expect("event");
            }
            events.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                events.len(),
                events
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });
        let provider = AnthropicProvider::new(AnthropicConfig::with_credential(
            format!("http://{address}/v1/"),
            "claude-test",
            SecretValue::new("test-key"),
        ))
        .expect("provider");
        let models = provider.list_models().await.expect("models");
        assert_eq!(models[0].id, "claude-test");
        assert_eq!(models[0].source, linguamesh_domain::ModelSource::Manual);
        let mut stream = provider
            .translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "claude-test"),
                CancellationToken::new(),
            )
            .await
            .expect("stream");
        let mut output = String::new();
        while let Some(delta) = stream.next().await {
            if let linguamesh_provider_api::TranslationStreamEvent::Text(text) =
                delta.expect("delta")
            {
                output.push_str(&text);
            }
        }
        assert_eq!(output, "你好，Anthropic！");
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn cancellation_interrupts_response_header_wait() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let stalled_server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("connection");
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let provider = AnthropicProvider::new(AnthropicConfig::without_credential(
            format!("http://{address}/v1/"),
            "claude-test",
        ))
        .expect("provider");
        let cancellation = CancellationToken::new();
        let cancellation_request = cancellation.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            cancellation_request.cancel();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            provider.translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "claude-test"),
                cancellation,
            ),
        )
        .await
        .expect("cancellation timeout");
        let Err(error) = result else {
            panic!("cancelled request returned a stream");
        };
        assert_eq!(error.kind, ErrorKind::Cancelled);
        cancel_task.await.expect("cancel task");
        stalled_server.abort();
        let _ = stalled_server.await;
    }
}
