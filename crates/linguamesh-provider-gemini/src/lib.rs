#![doc = "Google Gemini Generate Content 提供商适配器。"]

use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use linguamesh_domain::{
    ChunkingError, DEFAULT_TRANSLATION_CHUNK_BYTES, EndpointConfiguration, ErrorKind,
    ModelDescriptor, ModelSource, ProtectedSource, ProtectedTextError, SecretValue,
    TranslationError, TranslationRequest, protect_source_text_with_glossary,
};
use linguamesh_provider_api::{ModelProvider, TranslationStream, translation_prompt};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// 配置 Gemini Generate Content 端点。
pub struct GeminiConfig {
    /// 通常以 `/v1beta/` 结尾的基础地址。
    pub base_url: String,
    /// 可选的一次性内存凭据。
    pub credential: Option<SecretValue>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
}

impl GeminiConfig {
    /// 创建没有凭据的本地或测试配置。
    #[must_use]
    pub fn without_credential(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credential: None,
            request_timeout: Duration::from_secs(30),
        }
    }

    /// 创建携带一次性内存凭据的配置。
    #[must_use]
    pub fn with_credential(base_url: impl Into<String>, credential: SecretValue) -> Self {
        Self {
            base_url: base_url.into(),
            credential: Some(credential),
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl fmt::Debug for GeminiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiConfig")
            .field("base_url", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// 实现 Gemini 模型发现和 Generate Content 流式翻译。
#[derive(Clone)]
pub struct GeminiProvider {
    client: Client,
    base_url: Url,
    credential: Arc<Mutex<CredentialState>>,
    session_cancellation: CancellationToken,
}

enum CredentialState {
    NotRequired,
    Available(SecretValue),
    Cleared,
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .credential
            .lock()
            .map_or("poisoned", |credential| match &*credential {
                CredentialState::NotRequired => "not_required",
                CredentialState::Available(_) => "available_redacted",
                CredentialState::Cleared => "cleared",
            });
        formatter
            .debug_struct("GeminiProvider")
            .field("base_url", &"[REDACTED]")
            .field("credential_state", &state)
            .field("session_closed", &self.session_cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl GeminiProvider {
    /// 在请求宿主秘密之前验证端点策略。
    pub fn validate_endpoint(base_url: &str) -> Result<(), TranslationError> {
        validated_base_url(base_url).map(|_| ())
    }

    /// 创建拒绝跨源重定向的适配器。
    pub fn new(config: GeminiConfig) -> Result<Self, TranslationError> {
        let base_url = validated_base_url(&config.base_url)?;
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .timeout(config.request_timeout);
        if base_url.scheme() == "http" {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|error| map_reqwest_error(&error))?;
        Ok(Self {
            client,
            base_url,
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
            Err(poisoned) => *poisoned.into_inner() = CredentialState::Cleared,
        }
    }

    fn request(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, TranslationError> {
        if self.session_cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
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
                Ok(request.header("x-goog-api-key", secret.expose_secret()))
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
        let model = valid_model_id(&request.model_id)?;
        let marker_instruction = if protected.is_empty() {
            String::new()
        } else {
            " Preserve every opaque marker such as __LINGUAMESH_PROTECTED_0__ exactly once."
                .to_owned()
        };
        let mut span_restorer = protected.restorer();
        let body = GenerateRequest {
            contents: vec![GenerateContent {
                role: "user".to_owned(),
                parts: vec![GeneratePart {
                    text: format!(
                        "{}\n<source>\n{}\n</source>",
                        translation_prompt(
                            &request.target_locale,
                            request.quality_mode,
                            Some(&request.preset),
                            &marker_instruction,
                        ),
                        request.source_text
                    ),
                }],
            }],
        };
        let mut endpoint = self.endpoint(&format!("models/{model}:streamGenerateContent"))?;
        endpoint.query_pairs_mut().append_pair("alt", "sse");
        let request = self.request(self.client.post(endpoint))?.json(&body).send();
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
                let Some(chunk) = next else { break };
                let chunk = chunk.map_err(|error| map_reqwest_error(&error))?;
                total_bytes = total_bytes.saturating_add(chunk.len());
                if total_bytes > MAX_RESPONSE_BYTES {
                    Err(TranslationError::new(ErrorKind::MalformedResponse, "Provider stream exceeded the response-size limit."))?;
                }
                for message in decoder.push(&chunk)? {
                    match message {
                        SseMessage::Delta(text) if !text.is_empty() => {
                            let safe_delta = span_restorer.push(&text).map_err(|error| map_protected_error(&error))?;
                            if !safe_delta.is_empty() { yield safe_delta; }
                        }
                        SseMessage::Delta(_) => {}
                        SseMessage::DeltaAndDone(text) => {
                            if !text.is_empty() {
                                let safe_delta = span_restorer.push(&text).map_err(|error| map_protected_error(&error))?;
                                if !safe_delta.is_empty() { yield safe_delta; }
                            }
                            completed = true;
                            break;
                        }
                        SseMessage::Done => { completed = true; break; }
                    }
                }
                if completed { break; }
            }
            if !completed {
                Err(TranslationError::new(ErrorKind::MalformedResponse, "Provider stream ended before the completion marker."))?;
            }
            let safe_tail = span_restorer.finish().map_err(|error| map_protected_error(&error))?;
            if !safe_tail.is_empty() { yield safe_tail; }
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

fn valid_model_id(model_id: &str) -> Result<&str, TranslationError> {
    if model_id.is_empty()
        || model_id.len() > 128
        || !model_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "The Gemini model identifier is invalid.",
        ));
    }
    Ok(model_id)
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    async fn list_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError> {
        let response = tokio::select! {
            biased;
            () = self.session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            response = self.request(self.client.get(self.endpoint("models")?))?.send() => response.map_err(|error| map_reqwest_error(&error))?,
        };
        let response = ensure_success(response)?;
        let body: ModelListResponse = tokio::select! {
            biased;
            () = self.session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            body = response.json() => body.map_err(|error| map_reqwest_error(&error))?,
        };
        Ok(body
            .models
            .into_iter()
            .filter_map(|model| {
                let id = model
                    .name
                    .strip_prefix("models/")
                    .unwrap_or(&model.name)
                    .to_owned();
                let supports_generation = model
                    .supported_generation_methods
                    .iter()
                    .any(|method| method == "generateContent");
                supports_generation.then(|| ModelDescriptor {
                    display_name: model.display_name.unwrap_or_else(|| id.clone()),
                    id,
                    source: ModelSource::Discovered,
                })
            })
            .collect())
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
                let mut chunk_stream = provider.translate_protected_stream(chunk_request, chunk, cancellation.clone()).await?;
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
struct GenerateRequest {
    contents: Vec<GenerateContent>,
}

#[derive(Serialize, Deserialize)]
struct GenerateContent {
    role: String,
    parts: Vec<GeneratePart>,
}

#[derive(Serialize, Deserialize)]
struct GeneratePart {
    text: String,
}

#[derive(Deserialize)]
struct ModelListResponse {
    #[serde(default)]
    models: Vec<ModelResponse>,
}

#[derive(Deserialize)]
struct ModelResponse {
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
}

#[derive(Deserialize)]
struct StreamResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<ResponseContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    #[serde(default)]
    text: Option<String>,
}

enum SseMessage {
    Delta(String),
    DeltaAndDone(String),
    Done,
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
    let mut output = String::new();
    for part in response
        .candidates
        .iter()
        .filter_map(|candidate| candidate.content.as_ref())
        .flat_map(|content| content.parts.iter())
    {
        if let Some(text) = &part.text {
            output.push_str(text);
        }
    }
    if response
        .candidates
        .iter()
        .any(|candidate| candidate.finish_reason.is_some())
    {
        if output.is_empty() {
            return Ok(Some(SseMessage::Done));
        }
        return Ok(Some(SseMessage::DeltaAndDone(output)));
    }
    Ok(Some(SseMessage::Delta(output)))
}

fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, TranslationError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ErrorKind::Authentication,
        StatusCode::NOT_FOUND => ErrorKind::ModelUnavailable,
        _ => ErrorKind::Network,
    };
    Err(TranslationError::new(
        kind,
        format!("Provider request failed with HTTP status {status}."),
    ))
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
    use super::{GeminiConfig, GeminiProvider, SseDecoder, SseMessage};
    use bytes::Bytes;
    use futures_util::StreamExt;
    use linguamesh_domain::{ErrorKind, SecretValue, TranslationRequest};
    use linguamesh_provider_api::ModelProvider;
    use std::fmt::Write;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn decoder_handles_fragmented_utf8_and_finish_reason() {
        let payload = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"你好\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"！\"}]},\"finishReason\":\"STOP\"}]}\n\n",
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
        assert!(matches!(&messages[0], SseMessage::Delta(text) if text == "你好"));
        assert!(matches!(&messages[1], SseMessage::DeltaAndDone(text) if text == "！"));
    }

    #[test]
    fn diagnostics_redact_endpoint_and_credential() {
        const SECRET_CANARY: &str = concat!("g", "emini-LM_DEBUG_SECRET_123456");
        const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/";
        let config = GeminiConfig::with_credential(ENDPOINT, SecretValue::new(SECRET_CANARY));
        assert!(!format!("{config:?}").contains(SECRET_CANARY));
        assert!(!format!("{config:?}").contains(ENDPOINT));
        let provider = GeminiProvider::new(config).expect("provider");
        assert!(!format!("{provider:?}").contains(SECRET_CANARY));
        assert!(!format!("{provider:?}").contains(ENDPOINT));
        assert_eq!(
            super::valid_model_id("gemini-2.5-flash").expect("model"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            super::valid_model_id("models/gemini")
                .expect_err("invalid")
                .kind,
            ErrorKind::InvalidConfiguration
        );
    }

    #[tokio::test]
    async fn model_discovery_and_stream_use_gemini_wire_contract() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut list_socket, _) = listener.accept().await.expect("list connection");
            let list_body = r#"{"models":[{"name":"models/gemini-test","displayName":"Gemini Test","supportedGenerationMethods":["generateContent"]},{"name":"models/embed-test","supportedGenerationMethods":["embedContent"]}]}"#;
            let list_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                list_body.len(),
                list_body
            );
            list_socket
                .write_all(list_response.as_bytes())
                .await
                .expect("list response");
            let (mut stream_socket, _) = listener.accept().await.expect("stream connection");
            let mut events = String::new();
            for text in ["你好", "，Gemini"] {
                writeln!(
                    &mut events,
                    "data: {}\n",
                    serde_json::json!({"candidates":[{"content":{"parts":[{"text":text}]}}]})
                )
                .expect("event");
            }
            writeln!(
                &mut events,
                "data: {}\n",
                serde_json::json!({"candidates":[{"finishReason":"STOP"}]})
            )
            .expect("finish");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                events.len(),
                events
            );
            stream_socket
                .write_all(response.as_bytes())
                .await
                .expect("stream response");
        });
        let provider = GeminiProvider::new(GeminiConfig::with_credential(
            format!("http://{address}/v1beta/"),
            SecretValue::new("test-key"),
        ))
        .expect("provider");
        let models = provider.list_models().await.expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-test");
        let mut stream = provider
            .translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "gemini-test"),
                CancellationToken::new(),
            )
            .await
            .expect("stream");
        let mut output = String::new();
        while let Some(delta) = stream.next().await {
            output.push_str(&delta.expect("delta"));
        }
        assert_eq!(output, "你好，Gemini");
        server.await.expect("server task");
    }
}
