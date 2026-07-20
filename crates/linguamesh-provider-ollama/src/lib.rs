#![doc = "原生 Ollama `/api` 提供商适配器。"]

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

/// 配置原生 Ollama `/api` 端点。
pub struct OllamaConfig {
    /// 通常以 `/api/` 结尾的基础地址。
    pub base_url: String,
    /// 可选的内存凭据；原生 Ollama 默认不要求凭据。
    pub credential: Option<SecretValue>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
}

impl OllamaConfig {
    /// 创建没有凭据的本地配置。
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

impl fmt::Debug for OllamaConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OllamaConfig")
            .field("base_url", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// 实现原生 Ollama 模型发现和 NDJSON 流式聊天。
#[derive(Clone)]
pub struct OllamaProvider {
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

impl fmt::Debug for OllamaProvider {
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
            .debug_struct("OllamaProvider")
            .field("base_url", &"[REDACTED]")
            .field("credential_state", &credential_state)
            .field("session_closed", &self.session_cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl OllamaProvider {
    /// 在请求宿主秘密之前验证端点策略。
    pub fn validate_endpoint(base_url: &str) -> Result<(), TranslationError> {
        validated_base_url(base_url).map(|_| ())
    }

    /// 创建拒绝跨源重定向的适配器。
    pub fn new(config: OllamaConfig) -> Result<Self, TranslationError> {
        let base_url = validated_base_url(&config.base_url)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(config.request_timeout)
            .build()
            .map_err(|error| map_reqwest_error(&error))?;
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

    /// 取消该会话的请求并清除共享引用中的凭据。
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
        let mut credential = self.credential.lock().map_err(|poisoned| {
            *poisoned.into_inner() = CredentialState::Cleared;
            TranslationError::new(
                ErrorKind::Internal,
                "The provider credential session is unavailable.",
            )
        })?;
        match &mut *credential {
            CredentialState::NotRequired => Ok(request),
            CredentialState::Available(secret) => Ok(request.bearer_auth(secret.expose_secret())),
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
        let body = OllamaChatRequest {
            model: request.model_id,
            stream: true,
            messages: vec![
                OllamaMessage {
                    role: "system",
                    content: translation_prompt(
                        &request.target_locale,
                        request.quality_mode,
                        Some(&request.preset),
                        &marker_instruction,
                    ),
                },
                OllamaMessage {
                    role: "user",
                    content: format!("<source>\n{}\n</source>", request.source_text),
                },
            ],
        };
        let request = self
            .request(self.client.post(self.endpoint("chat")?))?
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
            let mut decoder = NdjsonDecoder::default();
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
                    Err(TranslationError::new(
                        ErrorKind::MalformedResponse,
                        "Provider stream exceeded the response-size limit.",
                    ))?;
                }
                for message in decoder.push(&chunk)? {
                    if let Some(text) = message.content {
                        let safe_delta = span_restorer.push(&text).map_err(|error| map_protected_error(&error))?;
                        if !safe_delta.is_empty() { yield safe_delta; }
                    }
                    if message.done { completed = true; break; }
                }
                if completed { break; }
            }
            if !completed {
                for message in decoder.finish()? {
                    if let Some(text) = message.content {
                        let safe_delta = span_restorer.push(&text).map_err(|error| map_protected_error(&error))?;
                        if !safe_delta.is_empty() { yield safe_delta; }
                    }
                    if message.done { completed = true; break; }
                }
            }
            if !completed {
                Err(TranslationError::new(
                    ErrorKind::MalformedResponse,
                    "Provider stream ended before the completion marker.",
                ))?;
            }
            let safe_tail = span_restorer.finish().map_err(|error| map_protected_error(&error))?;
            if !safe_tail.is_empty() { yield safe_tail; }
        };
        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    async fn list_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError> {
        let request = self
            .request(self.client.get(self.endpoint("tags")?))?
            .send();
        let response = tokio::select! {
            biased;
            () = self.session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            response = request => response.map_err(|error| map_reqwest_error(&error))?,
        };
        let response = ensure_success(response)?;
        let body: TagsResponse = response
            .json()
            .await
            .map_err(|error| map_reqwest_error(&error))?;
        Ok(body
            .models
            .into_iter()
            .filter_map(|model| {
                let id = model.name.or(model.model)?;
                (!id.trim().is_empty()).then_some(ModelDescriptor {
                    display_name: id.clone(),
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

fn map_chunking_error(error: ChunkingError) -> TranslationError {
    TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
}

fn map_protected_error(error: &ProtectedTextError) -> TranslationError {
    TranslationError::new(ErrorKind::MalformedResponse, error.to_string())
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    stream: bool,
    messages: Vec<OllamaMessage>,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct OllamaStreamResponse {
    #[serde(default)]
    message: Option<OllamaMessageResponse>,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize)]
struct OllamaMessageResponse {
    content: Option<String>,
}

struct OllamaStreamMessage {
    content: Option<String>,
    done: bool,
}

#[derive(Default)]
struct NdjsonDecoder {
    buffer: Vec<u8>,
}

impl NdjsonDecoder {
    fn push(&mut self, chunk: &Bytes) -> Result<Vec<OllamaStreamMessage>, TranslationError> {
        self.buffer.extend_from_slice(chunk);
        self.drain_lines(false)
    }

    fn finish(&mut self) -> Result<Vec<OllamaStreamMessage>, TranslationError> {
        self.drain_lines(true)
    }

    fn drain_lines(
        &mut self,
        final_line: bool,
    ) -> Result<Vec<OllamaStreamMessage>, TranslationError> {
        let mut output = Vec::new();
        loop {
            let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                if !final_line {
                    break;
                }
                if self.buffer.is_empty() {
                    break;
                }
                let line = std::mem::take(&mut self.buffer);
                output.push(parse_line(&line)?);
                break;
            };
            let mut line = self.buffer.drain(..=position).collect::<Vec<_>>();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                output.push(parse_line(&line)?);
            }
        }
        Ok(output)
    }
}

fn parse_line(line: &[u8]) -> Result<OllamaStreamMessage, TranslationError> {
    let response: OllamaStreamResponse = serde_json::from_slice(line).map_err(|_| {
        TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider stream contained malformed JSON.",
        )
    })?;
    Ok(OllamaStreamMessage {
        content: response.message.and_then(|message| message.content),
        done: response.done,
    })
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
    use super::{NdjsonDecoder, OllamaConfig, OllamaProvider};
    use bytes::Bytes;
    use futures_util::StreamExt;
    use linguamesh_domain::{ErrorKind, SecretValue, TranslationRequest};
    use linguamesh_provider_api::ModelProvider;
    use linguamesh_testkit::FakeProviderServer;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn decoder_handles_fragmented_utf8_and_completion() {
        let payload = "{\"message\":{\"content\":\"你好\"},\"done\":false}\n{\"done\":true}\n";
        let split = payload.find('好').expect("unicode split") + 1;
        let mut decoder = NdjsonDecoder::default();
        assert!(
            decoder
                .push(&Bytes::copy_from_slice(&payload.as_bytes()[..split]))
                .expect("first")
                .is_empty()
        );
        let messages = decoder
            .push(&Bytes::copy_from_slice(&payload.as_bytes()[split..]))
            .expect("second");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content.as_deref(), Some("你好"));
        assert!(!messages[0].done);
        assert!(messages[1].done);
    }

    #[test]
    fn diagnostics_redact_endpoint_and_credential() {
        const SECRET: &str = "ollama-secret-canary";
        const ENDPOINT: &str = "http://127.0.0.1:11434/api/";
        let config = OllamaConfig::with_credential(ENDPOINT, SecretValue::new(SECRET));
        assert!(!format!("{config:?}").contains(SECRET));
        let provider = OllamaProvider::new(config).expect("provider");
        assert!(!format!("{provider:?}").contains(SECRET));
        assert!(!format!("{provider:?}").contains(ENDPOINT));
    }

    #[tokio::test]
    async fn native_fixture_discovers_and_streams() {
        let server = FakeProviderServer::start_ollama_native()
            .await
            .expect("server");
        let provider =
            OllamaProvider::new(OllamaConfig::without_credential(server.ollama_base_url()))
                .expect("provider");
        let models = provider.list_models().await.expect("models");
        assert_eq!(models[0].id, "llama3.2:latest");
        let mut stream = provider
            .translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "llama3.2:latest"),
                CancellationToken::new(),
            )
            .await
            .expect("stream");
        let mut output = String::new();
        while let Some(delta) = stream.next().await {
            output.push_str(&delta.expect("delta"));
        }
        assert_eq!(output, "你好，Ollama！");
        server.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_is_reported_before_native_response_headers() {
        let provider =
            OllamaProvider::new(OllamaConfig::without_credential("http://127.0.0.1:1/api/"))
                .expect("provider");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = provider
            .translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "model"),
                cancellation,
            )
            .await;
        let Err(error) = result else {
            panic!("cancelled request returned a stream");
        };
        assert_eq!(error.kind, ErrorKind::Cancelled);
    }
}
