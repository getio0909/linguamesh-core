#![doc = "`LinguaMesh` 的提供商配置和宿主服务编排层。"]

use linguamesh_domain::{
    ErrorKind, HostRequestId, ModelDescriptor, ProviderProfile, ProviderProfileId, SecretRef,
    SecretValue, TranslationError,
};
use linguamesh_engine::TranslationEngine;
use linguamesh_provider_anthropic::{AnthropicConfig, AnthropicProvider};
use linguamesh_provider_api::ModelProvider;
use linguamesh_provider_gemini::{GeminiConfig, GeminiProvider};
use linguamesh_provider_ollama::{OllamaConfig, OllamaProvider};
use linguamesh_provider_openai::{
    AzureOpenAiConfig, OpenAiCompatibleProvider, OpenAiConfig, OpenAiResponsesConfig,
};
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// 默认宿主秘密请求队列容量。
pub const DEFAULT_SECRET_REQUEST_CAPACITY: usize = 8;

/// 描述核心向原生宿主发出的非秘密请求元数据。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretRequired {
    /// 关联一次宿主响应的不可预测标识。
    pub request_id: HostRequestId,
    /// 原生安全存储需要解析的引用。
    pub secret_ref: SecretRef,
}

/// 表示宿主无法回送一次秘密响应。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostResponseError;

impl fmt::Display for HostResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The Core secret request is no longer active.")
    }
}

impl Error for HostResponseError {}

/// 限制宿主只能提供秘密或返回预定义安全失败。
pub struct SecretRequestLease {
    required: SecretRequired,
    response: Option<oneshot::Sender<SecretResolution>>,
}

impl SecretRequestLease {
    /// 返回不含秘密值的请求元数据。
    #[must_use]
    pub const fn required(&self) -> &SecretRequired {
        &self.required
    }

    /// 一次性把内存秘密交给等待中的核心操作。
    pub fn provide_secret(mut self, secret: SecretValue) -> Result<(), HostResponseError> {
        self.respond(SecretResolution::Provided(secret))
    }

    /// 明确报告引用不存在或当前无法访问。
    pub fn reject_unavailable(mut self) -> Result<(), HostResponseError> {
        self.respond(SecretResolution::Unavailable)
    }

    /// 明确报告当前桌面没有安全持久化服务。
    pub fn reject_secure_storage_unavailable(mut self) -> Result<(), HostResponseError> {
        self.respond(SecretResolution::SecureStorageUnavailable)
    }

    fn is_active(&self) -> bool {
        self.response
            .as_ref()
            .is_some_and(|response| !response.is_closed())
    }

    fn respond(&mut self, resolution: SecretResolution) -> Result<(), HostResponseError> {
        self.response
            .take()
            .ok_or(HostResponseError)?
            .send(resolution)
            .map_err(|_| HostResponseError)
    }
}

impl fmt::Debug for SecretRequestLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretRequestLease")
            .field("required", &self.required)
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

/// 供原生宿主在独立执行上下文接收秘密请求。
pub struct HostSecretRequests {
    requests: mpsc::Receiver<SecretRequestLease>,
}

impl HostSecretRequests {
    /// 等待下一条仍活动的秘密请求并丢弃已取消积压。
    pub async fn recv(&mut self) -> Option<SecretRequestLease> {
        while let Some(request) = self.requests.recv().await {
            if request.is_active() {
                return Some(request);
            }
        }
        None
    }

    /// 返回创建通道时固定的最大容量。
    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.requests.max_capacity()
    }
}

/// 供核心操作发出有界且可取消的秘密请求。
#[derive(Clone)]
pub struct HostSecretBroker {
    requests: mpsc::Sender<SecretRequestLease>,
}

impl HostSecretBroker {
    /// 请求宿主解析引用，并让取消优先于排队和响应。
    pub async fn resolve(
        &self,
        secret_ref: &SecretRef,
        cancellation: &CancellationToken,
    ) -> Result<SecretValue, TranslationError> {
        let (response, receiver) = oneshot::channel();
        let lease = SecretRequestLease {
            required: SecretRequired {
                request_id: HostRequestId::new(),
                secret_ref: secret_ref.clone(),
            },
            response: Some(response),
        };
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TranslationError::cancelled()),
            result = self.requests.send(lease) => result.map_err(|_| TranslationError::new(
                ErrorKind::SecretUnavailable,
                "The host secret service is unavailable.",
            ))?,
        }
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(TranslationError::cancelled()),
            response = receiver => {
                if cancellation.is_cancelled() {
                    Err(TranslationError::cancelled())
                } else {
                    match response {
                        Ok(SecretResolution::Provided(secret)) => Ok(secret),
                        Ok(SecretResolution::Unavailable) => Err(TranslationError::new(
                            ErrorKind::SecretUnavailable,
                            "The provider credential is unavailable.",
                        )),
                        Ok(SecretResolution::SecureStorageUnavailable) => Err(TranslationError::new(
                            ErrorKind::SecureStorageUnavailable,
                            "Secure credential storage is unavailable.",
                        )),
                        Err(_) => Err(TranslationError::new(
                            ErrorKind::SecretUnavailable,
                            "The host did not complete the secret request.",
                        )),
                    }
                }
            }
        }
    }
}

enum SecretResolution {
    Provided(SecretValue),
    Unavailable,
    SecureStorageUnavailable,
}

/// 创建容量明确的秘密请求通道。
pub fn host_secret_channel(
    capacity: usize,
) -> Result<(HostSecretBroker, HostSecretRequests), TranslationError> {
    if capacity == 0 {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "The host secret request capacity must be greater than zero.",
        ));
    }
    let (requests, receiver) = mpsc::channel(capacity);
    Ok((
        HostSecretBroker { requests },
        HostSecretRequests { requests: receiver },
    ))
}

/// 管理每个应用实例唯一的活动提供商和可清理秘密会话。
pub struct ProviderManager {
    secret_broker: HostSecretBroker,
    active: Option<ActiveProvider>,
}

impl ProviderManager {
    /// 绑定由当前原生客户端拥有的秘密服务。
    #[must_use]
    pub const fn new(secret_broker: HostSecretBroker) -> Self {
        Self {
            secret_broker,
            active: None,
        }
    }

    /// 连接候选配置，并只在成功且未取消时替换活动会话。
    #[allow(clippy::too_many_lines)]
    pub async fn connect(
        &mut self,
        profile: &ProviderProfile,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ModelDescriptor>, TranslationError> {
        if !profile.enabled() {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "The provider profile is disabled.",
            ));
        }
        let is_ollama = profile.adapter_type() == "ollama_chat";
        let is_anthropic = profile.adapter_type() == "anthropic_messages";
        let is_gemini = profile.adapter_type() == "gemini_generate_content";
        let is_azure = profile.adapter_type() == "azure_openai_chat";
        let is_responses = profile.adapter_type() == "openai_responses";
        if !is_ollama
            && !is_anthropic
            && !is_gemini
            && !is_azure
            && !is_responses
            && profile.adapter_type() != "openai_chat_completions"
        {
            return Err(TranslationError::new(
                ErrorKind::UnsupportedCapability,
                "The provider adapter is not supported by this Core build.",
            ));
        }
        if is_ollama {
            OllamaProvider::validate_endpoint(profile.base_endpoint())?;
        } else if is_anthropic {
            AnthropicProvider::validate_endpoint(profile.base_endpoint())?;
        } else if is_gemini {
            GeminiProvider::validate_endpoint(profile.base_endpoint())?;
        } else if is_azure {
            let deployment = profile.selected_model().ok_or_else(|| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Select an Azure OpenAI deployment before connecting.",
                )
            })?;
            OpenAiCompatibleProvider::validate_azure_endpoint(
                profile.base_endpoint(),
                deployment,
                "2024-10-21",
            )?;
        } else {
            OpenAiCompatibleProvider::validate_endpoint(profile.base_endpoint())?;
        }
        let manual_model_id = if is_anthropic || is_azure {
            Some(
                profile
                    .selected_model()
                    .ok_or_else(|| {
                        TranslationError::new(
                            ErrorKind::InvalidConfiguration,
                            if is_azure {
                                "Select an Azure OpenAI deployment before connecting."
                            } else {
                                "Select a manual Anthropic model before connecting."
                            },
                        )
                    })?
                    .to_owned(),
            )
        } else {
            None
        };
        let credential = match profile.secret_ref() {
            Some(secret_ref) => Some(self.secret_broker.resolve(secret_ref, cancellation).await?),
            None => None,
        };
        if cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
        let provider: Arc<dyn ManagedProvider> = if is_ollama {
            let config = match credential {
                Some(secret) => OllamaConfig::with_credential(profile.base_endpoint(), secret),
                None => OllamaConfig::without_credential(profile.base_endpoint()),
            };
            Arc::new(OllamaProvider::new(config)?)
        } else if is_anthropic {
            let model_id = manual_model_id.ok_or_else(|| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Select a manual Anthropic model before connecting.",
                )
            })?;
            let config = match credential {
                Some(secret) => {
                    AnthropicConfig::with_credential(profile.base_endpoint(), model_id, secret)
                }
                None => AnthropicConfig::without_credential(profile.base_endpoint(), model_id),
            };
            Arc::new(AnthropicProvider::new(config)?)
        } else if is_gemini {
            let config = match credential {
                Some(secret) => GeminiConfig::with_credential(profile.base_endpoint(), secret),
                None => GeminiConfig::without_credential(profile.base_endpoint()),
            };
            Arc::new(GeminiProvider::new(config)?)
        } else if is_azure {
            let deployment = manual_model_id.ok_or_else(|| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Select an Azure OpenAI deployment before connecting.",
                )
            })?;
            let config = match credential {
                Some(secret) => AzureOpenAiConfig::with_credential(
                    profile.base_endpoint(),
                    deployment,
                    "2024-10-21",
                    secret,
                ),
                None => AzureOpenAiConfig::without_credential(
                    profile.base_endpoint(),
                    deployment,
                    "2024-10-21",
                ),
            };
            Arc::new(OpenAiCompatibleProvider::new_azure(config)?)
        } else if is_responses {
            let config = match credential {
                Some(secret) => {
                    OpenAiResponsesConfig::with_credential(profile.base_endpoint(), secret)
                }
                None => OpenAiResponsesConfig::without_credential(profile.base_endpoint()),
            }
            .with_organization(profile.organization().map(str::to_owned))
            .with_project(profile.project().map(str::to_owned))
            .with_custom_headers(profile.custom_headers().map(str::to_owned));
            Arc::new(OpenAiCompatibleProvider::new_responses(config)?)
        } else {
            let config = match credential {
                Some(secret) => OpenAiConfig::with_credential(profile.base_endpoint(), secret),
                None => OpenAiConfig::without_credential(profile.base_endpoint()),
            }
            .with_organization(profile.organization().map(str::to_owned))
            .with_project(profile.project().map(str::to_owned))
            .with_custom_headers(profile.custom_headers().map(str::to_owned));
            Arc::new(OpenAiCompatibleProvider::new(config)?)
        };
        let engine_provider: Arc<dyn ModelProvider> = provider.clone();
        let engine = TranslationEngine::new(engine_provider);
        let models = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TranslationError::cancelled()),
            models = engine.list_models() => models?,
        };
        if cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
        if models.is_empty() {
            return Err(TranslationError::new(
                ErrorKind::ModelUnavailable,
                "The provider returned no models.",
            ));
        }
        self.active = Some(ActiveProvider {
            profile_id: profile.id().clone(),
            provider,
            engine,
            models: models.clone(),
        });
        Ok(models)
    }

    /// 返回当前活动提供商的共享引擎。
    #[must_use]
    pub fn active_engine(&self) -> Option<&TranslationEngine> {
        self.active.as_ref().map(|active| &active.engine)
    }

    /// 返回当前活动配置标识。
    #[must_use]
    pub fn active_profile_id(&self) -> Option<&ProviderProfileId> {
        self.active.as_ref().map(|active| &active.profile_id)
    }

    /// 返回当前连接阶段发现的模型。
    #[must_use]
    pub fn models(&self) -> &[ModelDescriptor] {
        self.active
            .as_ref()
            .map_or(&[], |active| active.models.as_slice())
    }

    /// 丢弃活动引擎并清除其唯一内存凭据会话。
    pub fn disconnect(&mut self) {
        self.active = None;
    }
}

impl fmt::Debug for ProviderManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderManager")
            .field("has_active_provider", &self.active.is_some())
            .field("model_count", &self.models().len())
            .finish_non_exhaustive()
    }
}

struct ActiveProvider {
    profile_id: ProviderProfileId,
    provider: Arc<dyn ManagedProvider>,
    engine: TranslationEngine,
    models: Vec<ModelDescriptor>,
}

impl Drop for ActiveProvider {
    fn drop(&mut self) {
        self.provider.close_session();
    }
}

trait ManagedProvider: ModelProvider {
    fn close_session(&self);
}

impl ManagedProvider for OpenAiCompatibleProvider {
    fn close_session(&self) {
        Self::close_session(self);
    }
}

impl ManagedProvider for OllamaProvider {
    fn close_session(&self) {
        Self::close_session(self);
    }
}

impl ManagedProvider for AnthropicProvider {
    fn close_session(&self) {
        Self::close_session(self);
    }
}

impl ManagedProvider for GeminiProvider {
    fn close_session(&self) {
        Self::close_session(self);
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderManager, host_secret_channel};
    use linguamesh_domain::{
        ErrorKind, ProviderProfile, ProviderProfileId, SecretRef, SecretValue, TranslationEvent,
        TranslationRequest,
    };
    use linguamesh_storage::Storage;
    use linguamesh_testkit::FakeProviderServer;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    const PROVIDER_SECRET_REF: &str = "secret-service:11111111-1111-4111-8111-111111111111";
    const CANCELLED_SECRET_REF: &str = "secret-service:22222222-2222-4222-8222-222222222222";
    const FIRST_SECRET_REF: &str = "secret-service:33333333-3333-4333-8333-333333333333";
    const SECOND_SECRET_REF: &str = "secret-service:44444444-4444-4444-8444-444444444444";
    const MISSING_SECRET_REF: &str = "secret-service:55555555-5555-4555-8555-555555555555";

    fn profile(endpoint: &str, secret_ref: Option<&str>) -> ProviderProfile {
        profile_with_adapter(
            endpoint,
            secret_ref,
            "local-loopback",
            "openai_chat_completions",
        )
    }

    fn profile_with_adapter(
        endpoint: &str,
        secret_ref: Option<&str>,
        preset_id: &str,
        adapter: &str,
    ) -> ProviderProfile {
        ProviderProfile::new(
            ProviderProfileId::parse("provider-profile").expect("profile id"),
            "Provider",
            preset_id,
            adapter,
            endpoint,
            secret_ref.map(|value| SecretRef::parse(value).expect("secret ref")),
        )
        .expect("profile")
    }

    #[tokio::test]
    async fn correlated_secret_resolves_authenticated_provider_without_disk_leakage() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_TEST_SECRET_CANARY_1234567890");
        let directory = tempdir().expect("temp directory");
        let database_path = directory.path().join("provider.sqlite3");
        let server =
            FakeProviderServer::start_requiring_bearer_token(SecretValue::new(SECRET_CANARY))
                .await
                .expect("server");
        let provider = profile(&server.base_url(), Some(PROVIDER_SECRET_REF));
        let mut storage = Storage::open(&database_path).expect("storage");
        storage
            .save_and_activate_provider(&provider)
            .expect("persist profile");

        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let host = tokio::spawn(async move {
            let request = requests.recv().await.expect("secret request");
            assert!(!request.required().request_id.as_str().is_empty());
            assert_eq!(request.required().secret_ref.as_str(), PROVIDER_SECRET_REF);
            let debug = format!("{request:?}");
            assert!(!debug.contains(SECRET_CANARY));
            request
                .provide_secret(SecretValue::new(SECRET_CANARY))
                .expect("provide secret");
        });
        let mut manager = ProviderManager::new(broker);
        let models = manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("authenticated connection");
        assert_eq!(models[0].id, "fake-translator");
        assert_eq!(manager.active_profile_id(), Some(provider.id()));
        host.await.expect("host task");
        manager.disconnect();
        assert!(manager.active_engine().is_none());

        let mut saw_wal = false;
        let mut saw_shm = false;
        for entry in fs::read_dir(directory.path()).expect("database directory") {
            let path = entry.expect("directory entry").path();
            if !path.is_file() {
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("artifact name");
            saw_wal |= file_name.ends_with("-wal");
            saw_shm |= file_name.ends_with("-shm");
            let bytes = fs::read(path).expect("database artifact");
            assert!(
                !bytes
                    .windows(SECRET_CANARY.len())
                    .any(|window| window == SECRET_CANARY.as_bytes())
            );
        }
        assert!(saw_wal);
        assert!(saw_shm);
        drop(storage);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn chat_profile_forwards_project_metadata_to_provider() {
        let server = FakeProviderServer::start_requiring_openai_project("project-manager")
            .await
            .expect("server");
        let provider = profile(&server.base_url(), None)
            .with_project(Some("project-manager".to_owned()))
            .expect("project metadata");
        let (broker, _requests) = host_secret_channel(1).expect("secret channel");
        let mut manager = ProviderManager::new(broker);
        let models = manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("project-authenticated connection");
        assert_eq!(models[0].id, "fake-translator");
        let mut operation = manager
            .active_engine()
            .expect("active engine")
            .translate(TranslationRequest::new("Hello", "zh-CN", "fake-translator"));
        let mut output = String::new();
        while let Some(event) = operation.next_event().await {
            match event {
                TranslationEvent::TextDelta { text, .. } => output.push_str(&text),
                TranslationEvent::Completed { .. } => break,
                TranslationEvent::Failed { error, .. } => {
                    panic!("Project-authenticated request failed: {error}")
                }
                _ => {}
            }
        }
        assert_eq!(output, "你好，LinguaMesh！");
        manager.disconnect();
        server.shutdown().await;
    }

    #[tokio::test]
    async fn native_ollama_profile_discovers_and_streams_without_a_secret() {
        let server = FakeProviderServer::start_ollama_native()
            .await
            .expect("server");
        let provider =
            profile_with_adapter(&server.ollama_base_url(), None, "ollama", "ollama_chat");
        let (broker, _requests) = host_secret_channel(1).expect("secret channel");
        let mut manager = ProviderManager::new(broker);
        let models = manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("Ollama connection");
        assert_eq!(models[0].id, "llama3.2:latest");
        let mut operation = manager
            .active_engine()
            .expect("active engine")
            .translate(TranslationRequest::new("Hello", "zh-CN", "llama3.2:latest"));
        let mut output = String::new();
        while let Some(event) = operation.next_event().await {
            match event {
                TranslationEvent::TextDelta { text, .. } => output.push_str(&text),
                TranslationEvent::Completed { .. } => break,
                TranslationEvent::Failed { error, .. } => panic!("Ollama failed: {error}"),
                _ => {}
            }
        }
        assert_eq!(output, "你好，Ollama！");
        manager.disconnect();
        server.shutdown().await;
    }

    #[tokio::test]
    async fn azure_profile_uses_api_key_and_manual_deployment() {
        let server = FakeProviderServer::start_azure().await.expect("server");
        let provider = ProviderProfile::new(
            ProviderProfileId::parse("azure-profile").expect("profile ID"),
            "Azure provider",
            "azure-openai",
            "azure_openai_chat",
            server.azure_base_url(),
            Some(SecretRef::parse(PROVIDER_SECRET_REF).expect("secret ref")),
        )
        .expect("profile")
        .with_selected_model(Some("fake-deployment".to_owned()))
        .expect("deployment");
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let host = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("secret request")
                .provide_secret(SecretValue::new("azure-test-key"))
                .expect("provide secret");
        });
        let mut manager = ProviderManager::new(broker);
        let models = manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("Azure connection");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fake-deployment");
        assert_eq!(models[0].source, linguamesh_domain::ModelSource::Manual);
        let mut operation = manager
            .active_engine()
            .expect("active engine")
            .translate(TranslationRequest::new("Hello", "zh-CN", "fake-deployment"));
        let mut output = String::new();
        while let Some(event) = operation.next_event().await {
            match event {
                TranslationEvent::TextDelta { text, .. } => output.push_str(&text),
                TranslationEvent::Completed { .. } => break,
                TranslationEvent::Failed { error, .. } => panic!("Azure failed: {error}"),
                _ => {}
            }
        }
        assert_eq!(output, "你好，Azure！");
        host.await.expect("host task");
        manager.disconnect();
        server.shutdown().await;
    }

    #[tokio::test]
    async fn responses_profile_uses_typed_sse_stream() {
        let server =
            FakeProviderServer::start_responses_requiring_openai_project("responses-project")
                .await
                .expect("server");
        let provider = profile_with_adapter(
            &server.base_url(),
            Some(PROVIDER_SECRET_REF),
            "openai-responses",
            "openai_responses",
        )
        .with_project(Some("responses-project".to_owned()))
        .expect("project metadata");
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let host = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("secret request")
                .provide_secret(SecretValue::new("responses-test-key"))
                .expect("provide secret");
        });
        let mut manager = ProviderManager::new(broker);
        let models = manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("Responses connection");
        assert_eq!(models[0].id, "fake-translator");
        let mut operation = manager
            .active_engine()
            .expect("active engine")
            .translate(TranslationRequest::new("Hello", "zh-CN", "fake-translator"));
        let mut output = String::new();
        while let Some(event) = operation.next_event().await {
            match event {
                TranslationEvent::TextDelta { text, .. } => output.push_str(&text),
                TranslationEvent::Completed { .. } => break,
                TranslationEvent::Failed { error, .. } => panic!("Responses failed: {error}"),
                _ => {}
            }
        }
        assert_eq!(output, "你好，Responses！");
        host.await.expect("host task");
        manager.disconnect();
        server.shutdown().await;
    }

    #[tokio::test]
    async fn anthropic_profile_requires_manual_model_before_secret_request() {
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let provider = profile_with_adapter(
            "https://api.anthropic.com/v1/",
            Some(PROVIDER_SECRET_REF),
            "anthropic",
            "anthropic_messages",
        );
        let error = ProviderManager::new(broker)
            .connect(&provider, &CancellationToken::new())
            .await
            .expect_err("missing manual model");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert_eq!(
            error.message,
            "Select a manual Anthropic model before connecting."
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(20), requests.recv()).await,
            Err(_) | Ok(None)
        ));
    }

    #[tokio::test]
    async fn secret_request_cancellation_drops_late_response() {
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let cancellation = CancellationToken::new();
        let pending_cancellation = cancellation.clone();
        let secret_ref = SecretRef::parse(CANCELLED_SECRET_REF).expect("secret ref");
        let pending =
            tokio::spawn(async move { broker.resolve(&secret_ref, &pending_cancellation).await });
        let request = requests.recv().await.expect("secret request");
        cancellation.cancel();
        let error = pending.await.expect("request task").expect_err("cancelled");
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert!(
            request
                .provide_secret(SecretValue::new(concat!(
                    "s",
                    "k",
                    "-LM_LATE_SECRET_CANARY_123456"
                )))
                .is_err()
        );
    }

    #[tokio::test]
    async fn cancelled_backlog_is_skipped_without_consuming_capacity() {
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        assert_eq!(requests.max_capacity(), 1);
        let first_cancellation = CancellationToken::new();
        let pending_cancellation = first_cancellation.clone();
        let first_broker = broker.clone();
        let first = tokio::spawn(async move {
            first_broker
                .resolve(
                    &SecretRef::parse(FIRST_SECRET_REF).expect("secret ref"),
                    &pending_cancellation,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while requests.requests.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first request queued");
        assert_eq!(requests.requests.len(), 1);
        first_cancellation.cancel();
        assert_eq!(
            first
                .await
                .expect("first task")
                .expect_err("cancelled")
                .kind,
            ErrorKind::Cancelled
        );

        let second_broker = broker.clone();
        let second = tokio::spawn(async move {
            second_broker
                .resolve(
                    &SecretRef::parse(SECOND_SECRET_REF).expect("secret ref"),
                    &CancellationToken::new(),
                )
                .await
        });
        let request = requests.recv().await.expect("active second request");
        assert_eq!(request.required().secret_ref.as_str(), SECOND_SECRET_REF);
        request
            .provide_secret(SecretValue::new("second-secret"))
            .expect("second response");
        assert_eq!(
            second
                .await
                .expect("second task")
                .expect("second secret")
                .expose_secret(),
            "second-secret"
        );
    }

    #[tokio::test]
    async fn unavailable_secret_is_typed_and_actionable() {
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let host = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("secret request")
                .reject_secure_storage_unavailable()
                .expect("reject request");
        });
        let error = ProviderManager::new(broker)
            .connect(
                &profile("http://127.0.0.1:11434/v1/", Some(MISSING_SECRET_REF)),
                &CancellationToken::new(),
            )
            .await
            .expect_err("secure storage failure");
        assert_eq!(error.kind, ErrorKind::SecureStorageUnavailable);
        assert_eq!(error.message, "Secure credential storage is unavailable.");
        host.await.expect("host task");
    }

    #[tokio::test]
    async fn adapter_rejection_precedes_any_secret_request() {
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let invalid = ProviderProfile::new(
            ProviderProfileId::parse("invalid-adapter").expect("profile id"),
            "Invalid adapter",
            "local-loopback",
            "unsupported_adapter",
            "http://127.0.0.1:11434/v1/",
            Some(SecretRef::parse(PROVIDER_SECRET_REF).expect("secret ref")),
        )
        .expect("profile");
        let mut manager = ProviderManager::new(broker);
        let error = manager
            .connect(&invalid, &CancellationToken::new())
            .await
            .expect_err("unsupported adapter");
        assert_eq!(error.kind, ErrorKind::UnsupportedCapability);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), requests.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn connection_is_cancelled_during_stalled_model_discovery() {
        let server = FakeProviderServer::start_with_model_delay(Duration::from_secs(30))
            .await
            .expect("server");
        let request_counter = server.model_request_counter();
        let (broker, _requests) = host_secret_channel(1).expect("secret channel");
        let cancellation = CancellationToken::new();
        let pending_cancellation = cancellation.clone();
        let endpoint = server.base_url();
        let pending = tokio::spawn(async move {
            ProviderManager::new(broker)
                .connect(&profile(&endpoint, None), &pending_cancellation)
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while request_counter.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("model request started");
        cancellation.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .expect("bounded cancellation")
            .expect("connection task")
            .expect_err("cancelled connection");
        assert_eq!(result.kind, ErrorKind::Cancelled);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn failed_switch_preserves_previous_active_provider() {
        let server = FakeProviderServer::start().await.expect("server");
        let (broker, _requests) = host_secret_channel(1).expect("secret channel");
        let mut manager = ProviderManager::new(broker);
        let initial = profile(&server.base_url(), None);
        manager
            .connect(&initial, &CancellationToken::new())
            .await
            .expect("initial connection");
        let invalid = ProviderProfile::new(
            ProviderProfileId::parse("invalid-provider").expect("profile id"),
            "Invalid provider",
            "local-loopback",
            "unsupported_adapter",
            server.base_url(),
            None,
        )
        .expect("invalid adapter profile");
        assert!(
            manager
                .connect(&invalid, &CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(manager.active_profile_id(), Some(initial.id()));
        assert!(!manager.models().is_empty());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn disconnect_cancels_in_flight_operation_and_clears_session() {
        const SECRET: &str = concat!("s", "k", "-LM_DISCONNECT_SECRET_1234567890");
        let server = FakeProviderServer::start_requiring_bearer_token(SecretValue::new(SECRET))
            .await
            .expect("server");
        let provider = profile(&server.base_url(), Some(PROVIDER_SECRET_REF));
        let (broker, mut requests) = host_secret_channel(1).expect("secret channel");
        let host = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("secret request")
                .provide_secret(SecretValue::new(SECRET))
                .expect("provide secret");
        });
        let mut manager = ProviderManager::new(broker);
        manager
            .connect(&provider, &CancellationToken::new())
            .await
            .expect("connection");
        host.await.expect("host task");

        let request_counter = server.chat_request_counter();
        let mut operation =
            manager
                .active_engine()
                .expect("active engine")
                .translate(TranslationRequest::new(
                    "Hello",
                    "zh-CN",
                    "fake-slow-translator",
                ));
        assert!(matches!(
            operation.next_event().await,
            Some(TranslationEvent::Started { .. })
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while request_counter.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("chat request started");

        manager.disconnect();
        assert!(manager.active_engine().is_none());
        let mut terminal = None;
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = operation.next_event().await {
                if event.is_terminal() {
                    assert!(terminal.is_none());
                    terminal = Some(event);
                }
            }
        })
        .await
        .expect("bounded disconnect cancellation");
        assert!(matches!(terminal, Some(TranslationEvent::Cancelled { .. })));
        server.shutdown().await;
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(host_secret_channel(0).is_err());
    }
}
