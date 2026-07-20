#![doc = "提供有界事件流和取消传播的翻译引擎。"]

use futures_util::StreamExt;
use linguamesh_domain::{
    CoreCompatibility, ErrorKind, ModelDescriptor, TranslationError, TranslationEvent,
    TranslationRequest,
};
use linguamesh_protocol::{ABI_VERSION_MAJOR, PROTOCOL_VERSION};
use linguamesh_provider_api::ModelProvider;
use linguamesh_provider_catalog::ProviderCatalog;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const EVENT_CAPACITY: usize = 32;

/// 当前共享核心语义版本。
pub const CORE_SEMANTIC_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 当前核心可供客户端协商的稳定功能集合。
pub const CORE_FEATURES: &[&str] = &[
    "cancellation_v1",
    "azure_openai_chat_v1",
    "openai_responses_v1",
    "compatibility_negotiation_v1",
    "file_lease_v1",
    "typed_rust_host_secret_broker_v1",
    "model_discovery_v1",
    "protected_spans_v1",
    "long_text_chunking_v1",
    "provider_profile_persistence_v1",
    "bounded_text_document_v1",
    "routing_planner_v1",
    "translation_quality_modes_v1",
    "translation_presets_v1",
    "streaming_text_v1",
    "text_translation_v1",
];

/// 返回客户端启动时必须校验的完整共享契约描述。
pub fn core_compatibility() -> Result<CoreCompatibility, TranslationError> {
    let catalog = ProviderCatalog::bundled().map_err(|error| {
        TranslationError::new(
            ErrorKind::Internal,
            format!("Bundled provider catalog is invalid: {error}"),
        )
    })?;
    Ok(CoreCompatibility {
        core_version: CORE_SEMANTIC_VERSION.into(),
        abi_major: ABI_VERSION_MAJOR,
        protocol_version: PROTOCOL_VERSION,
        provider_catalog_version: catalog.catalog_version,
        enabled_features: CORE_FEATURES
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect(),
    })
}

/// 运行提供商无关的翻译操作。
pub struct TranslationEngine {
    provider: Arc<dyn ModelProvider>,
}

impl TranslationEngine {
    /// 包装一个经过配置的提供商适配器。
    #[must_use]
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        Self { provider }
    }

    /// 列出提供商发现的模型。
    pub async fn list_models(
        &self,
    ) -> Result<Vec<ModelDescriptor>, linguamesh_domain::TranslationError> {
        self.provider.list_models().await
    }

    /// 启动操作并立即返回可取消的有界事件接收器。
    #[must_use]
    pub fn translate(&self, request: TranslationRequest) -> TranslationOperation {
        self.translate_with_sequence_offset(request, 0)
    }

    /// 启动操作并从给定序号开始产生事件。
    #[must_use]
    pub fn translate_with_sequence_offset(
        &self,
        request: TranslationRequest,
        sequence_offset: u64,
    ) -> TranslationOperation {
        let provider = Arc::clone(&self.provider);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel(EVENT_CAPACITY);
        tokio::spawn(async move {
            let mut sequence = sequence_offset;
            let validation_request = request.clone();
            let mut output = String::new();
            if sender
                .send(TranslationEvent::Started { sequence })
                .await
                .is_err()
            {
                worker_cancellation.cancel();
                return;
            }
            sequence += 1;
            if let Err(error) = validation_request.validate() {
                send_error_terminal(&sender, sequence, error).await;
                return;
            }
            let stream = provider
                .translate_stream(request, worker_cancellation.clone())
                .await;
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    send_error_terminal(&sender, sequence, error).await;
                    return;
                }
            };
            while let Some(delta) = stream.next().await {
                match delta {
                    Ok(text) => {
                        output.push_str(&text);
                        if sender
                            .send(TranslationEvent::TextDelta { sequence, text })
                            .await
                            .is_err()
                        {
                            worker_cancellation.cancel();
                            return;
                        }
                        sequence += 1;
                    }
                    Err(error) => {
                        send_error_terminal(&sender, sequence, error).await;
                        return;
                    }
                }
            }
            if let Err(error) = validate_translation_output(&validation_request, &output) {
                send_error_terminal(&sender, sequence, error).await;
                return;
            }
            let _ = sender.send(TranslationEvent::Completed { sequence }).await;
        });
        TranslationOperation {
            events: receiver,
            cancellation,
        }
    }
}

fn validate_translation_output(
    request: &TranslationRequest,
    output: &str,
) -> Result<(), TranslationError> {
    if request.source_text.trim().is_empty() {
        return Ok(());
    }
    if output.trim().is_empty() {
        return Err(TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider returned an empty translation.",
        ));
    }
    if output.contains('\u{FFFD}') {
        return Err(TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider returned invalid UTF-8 replacement characters.",
        ));
    }
    Ok(())
}

/// 提供可跨宿主控制线程复制的取消能力。
#[derive(Clone)]
pub struct CancellationHandle {
    cancellation: CancellationToken,
}

impl CancellationHandle {
    /// 从宿主服务连接阶段使用的令牌创建取消句柄。
    #[must_use]
    pub fn from_token(cancellation: CancellationToken) -> Self {
        Self { cancellation }
    }

    /// 请求底层提供商操作停止且不进行重试。
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

/// 允许客户端接收事件和传播取消。
pub struct TranslationOperation {
    events: mpsc::Receiver<TranslationEvent>,
    cancellation: CancellationToken,
}

impl TranslationOperation {
    /// 返回可由另一执行上下文持有的取消句柄。
    #[must_use]
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            cancellation: self.cancellation.clone(),
        }
    }

    /// 等待下一条按序事件。
    pub async fn next_event(&mut self) -> Option<TranslationEvent> {
        self.events.recv().await
    }

    /// 请求取消传输且不进行重试。
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

async fn send_error_terminal(
    sender: &mpsc::Sender<TranslationEvent>,
    sequence: u64,
    error: linguamesh_domain::TranslationError,
) {
    let event = if error.kind == ErrorKind::Cancelled {
        TranslationEvent::Cancelled { sequence }
    } else {
        TranslationEvent::Failed { sequence, error }
    };
    let _ = sender.send(event).await;
}

#[cfg(test)]
mod tests {
    use super::{
        CORE_FEATURES, CORE_SEMANTIC_VERSION, TranslationEngine, core_compatibility,
        validate_translation_output,
    };
    use linguamesh_domain::{ErrorKind, TranslationEvent, TranslationRequest};
    use linguamesh_protocol::{ABI_VERSION_MAJOR, PROTOCOL_VERSION};
    use linguamesh_provider_openai::{OpenAiCompatibleProvider, OpenAiConfig};
    use linguamesh_testkit::FakeProviderServer;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn compatibility_reports_every_required_contract_dimension() {
        let compatibility = core_compatibility().expect("compatibility");
        assert_eq!(compatibility.core_version, CORE_SEMANTIC_VERSION);
        assert_eq!(compatibility.abi_major, ABI_VERSION_MAJOR);
        assert_eq!(compatibility.protocol_version, PROTOCOL_VERSION);
        assert_eq!(compatibility.provider_catalog_version, "0.1.0");
        assert_eq!(
            compatibility.enabled_features,
            CORE_FEATURES
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn deterministic_output_validation_rejects_empty_and_replacement_text() {
        let request = TranslationRequest::new("Hello", "zh-CN", "model");
        let empty = validate_translation_output(&request, " ").expect_err("empty output");
        assert_eq!(empty.kind, ErrorKind::MalformedResponse);
        let replacement =
            validate_translation_output(&request, "\u{FFFD}").expect_err("replacement output");
        assert_eq!(replacement.kind, ErrorKind::MalformedResponse);
        assert!(validate_translation_output(&request, "你好").is_ok());
    }

    #[tokio::test]
    async fn real_http_stream_has_one_completed_terminal_event() {
        let server = FakeProviderServer::start().await.expect("server");
        let provider =
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(server.base_url()))
                .expect("provider");
        let engine = TranslationEngine::new(Arc::new(provider));
        let models = engine.list_models().await.expect("models");
        assert_eq!(models[0].id, "fake-translator");
        let mut operation =
            engine.translate(TranslationRequest::new("Hello", "zh-CN", "fake-translator"));
        let mut output = String::new();
        let mut terminals = 0;
        while let Some(event) = operation.next_event().await {
            match event {
                TranslationEvent::TextDelta { text, .. } => output.push_str(&text),
                event if event.is_terminal() => {
                    terminals += 1;
                    assert!(matches!(event, TranslationEvent::Completed { .. }));
                }
                _ => {}
            }
        }
        assert_eq!(output, "你好，LinguaMesh！");
        assert_eq!(terminals, 1);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_retains_partial_output_and_has_one_terminal_event() {
        let server = FakeProviderServer::start().await.expect("server");
        let provider =
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(server.base_url()))
                .expect("provider");
        let engine = TranslationEngine::new(Arc::new(provider));
        let mut operation = engine.translate(TranslationRequest::new(
            "Hello",
            "zh-CN",
            "fake-slow-translator",
        ));
        let first = operation.next_event().await.expect("started");
        assert!(matches!(first, TranslationEvent::Started { .. }));
        let first_delta = operation.next_event().await.expect("delta");
        let partial = match first_delta {
            TranslationEvent::TextDelta { text, .. } => text,
            other => panic!("expected delta, got {other:?}"),
        };
        operation.cancel();
        let terminal = tokio::time::timeout(Duration::from_secs(2), operation.next_event())
            .await
            .expect("terminal timeout")
            .expect("terminal");
        assert!(!partial.is_empty());
        assert!(matches!(terminal, TranslationEvent::Cancelled { .. }));
        assert!(operation.next_event().await.is_none());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn premature_disconnect_is_failed_not_completed() {
        let server = FakeProviderServer::start().await.expect("server");
        let provider =
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(server.base_url()))
                .expect("provider");
        let engine = TranslationEngine::new(Arc::new(provider));
        let mut operation = engine.translate(TranslationRequest::new(
            "[disconnect]",
            "zh-CN",
            "fake-translator",
        ));
        let mut terminal = None;
        while let Some(event) = operation.next_event().await {
            if event.is_terminal() {
                terminal = Some(event);
            }
        }
        assert!(matches!(terminal, Some(TranslationEvent::Failed { .. })));
        server.shutdown().await;
    }
}
