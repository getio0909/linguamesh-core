#![doc = "通用 `OpenAI` 兼容提供商适配器。"]

use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use linguamesh_domain::{
    ChunkingError, ClientCertificateIdentity, DEFAULT_TRANSLATION_CHUNK_BYTES,
    EndpointConfiguration, ErrorKind, ModelDescriptor, ModelSource, ProtectedSource,
    ProtectedTextError, ProxyAuthentication, SecretValue, TranslationError, TranslationRequest,
    UsageRecord, protect_source_text_with_glossary,
};
use linguamesh_provider_api::{
    ModelProvider, TranslationStream, TranslationStreamEvent, error_kind_for_http_status,
    retry_after_ms, translation_prompt,
};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CUSTOM_HEADERS: usize = 16;
const MAX_CUSTOM_HEADER_NAME_BYTES: usize = 128;
const MAX_CUSTOM_HEADER_VALUE_BYTES: usize = 2048;

/// 兼容旧预发布调用方的凭据类型别名。
#[deprecated(note = "Use linguamesh_domain::SecretValue.")]
pub type ApiCredential = SecretValue;

/// 配置通用 `OpenAI` 兼容端点。
pub struct OpenAiConfig {
    /// 通常以 `/v1/` 结尾的基础地址。
    pub base_url: String,
    /// 可选的内存凭据。
    pub credential: Option<SecretValue>,
    /// 可选的非秘密组织标识。
    pub organization: Option<String>,
    /// 可选的非秘密项目标识。
    pub project: Option<String>,
    /// 可选的受限非秘密请求头 JSON。
    pub custom_headers: Option<String>,
    /// 可选的一次性内存秘密请求头 JSON。
    pub secret_custom_headers: Option<SecretValue>,
    /// 可选的不含凭据代理地址。
    pub proxy_url: Option<String>,
    /// 可选的一次性内存代理认证。
    pub proxy_authentication: Option<SecretValue>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
    /// 建立网络连接的超时。
    pub connection_timeout: Duration,
    /// 流式响应等待下一数据块的超时。
    pub streaming_idle_timeout: Duration,
    /// 可选的自定义可信证书 PEM；不会关闭 TLS 校验。
    pub trusted_certificates_pem: Option<String>,
    /// 可选的一次性内存 TLS 客户端证书身份。
    pub client_certificate_identity: Option<ClientCertificateIdentity>,
}

/// 配置 Azure `OpenAI` Chat Completions 部署端点。
pub struct AzureOpenAiConfig {
    /// Azure `OpenAI` 资源根地址，例如 `https://resource.openai.azure.com/`。
    pub base_url: String,
    /// Azure 部署名；该值也作为用户可选择的手工模型标识。
    pub deployment: String,
    /// Azure API 版本查询参数。
    pub api_version: String,
    /// 可选的一次性内存凭据。
    pub credential: Option<SecretValue>,
    /// 可选的受限非秘密请求头 JSON。
    pub custom_headers: Option<String>,
    /// 可选的一次性内存秘密请求头 JSON。
    pub secret_custom_headers: Option<SecretValue>,
    /// 可选的不含凭据代理地址。
    pub proxy_url: Option<String>,
    /// 可选的一次性内存代理认证。
    pub proxy_authentication: Option<SecretValue>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
    /// 建立网络连接的超时。
    pub connection_timeout: Duration,
    /// 流式响应等待下一数据块的超时。
    pub streaming_idle_timeout: Duration,
    /// 可选的自定义可信证书 PEM；不会关闭 TLS 校验。
    pub trusted_certificates_pem: Option<String>,
    /// 可选的一次性内存 TLS 客户端证书身份。
    pub client_certificate_identity: Option<ClientCertificateIdentity>,
}

impl AzureOpenAiConfig {
    /// 创建没有凭据的本地或测试配置。
    #[must_use]
    pub fn without_credential(
        base_url: impl Into<String>,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            deployment: deployment.into(),
            api_version: api_version.into(),
            credential: None,
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 创建携带一次性内存凭据的配置。
    #[must_use]
    pub fn with_credential(
        base_url: impl Into<String>,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
        credential: SecretValue,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            deployment: deployment.into(),
            api_version: api_version.into(),
            credential: Some(credential),
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 设置受限非秘密请求头 JSON。
    #[must_use]
    pub fn with_custom_headers(mut self, custom_headers: Option<String>) -> Self {
        self.custom_headers = custom_headers;
        self
    }

    /// 设置不含凭据的代理地址。
    #[must_use]
    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }

    /// 设置一次性内存代理认证。
    #[must_use]
    pub fn with_proxy_authentication(mut self, proxy_authentication: Option<SecretValue>) -> Self {
        self.proxy_authentication = proxy_authentication;
        self
    }

    /// 设置请求总超时。
    #[must_use]
    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// 设置连接建立超时。
    #[must_use]
    pub fn with_connection_timeout(mut self, connection_timeout: Duration) -> Self {
        self.connection_timeout = connection_timeout;
        self
    }

    /// 设置流式响应空闲超时。
    #[must_use]
    pub fn with_streaming_idle_timeout(mut self, streaming_idle_timeout: Duration) -> Self {
        self.streaming_idle_timeout = streaming_idle_timeout;
        self
    }

    /// 设置自定义可信证书 PEM；系统证书仍然保留。
    #[must_use]
    pub fn with_trusted_certificates_pem(
        mut self,
        trusted_certificates_pem: Option<String>,
    ) -> Self {
        self.trusted_certificates_pem = trusted_certificates_pem;
        self
    }

    /// 设置一次性内存 TLS 客户端证书身份。
    #[must_use]
    pub fn with_client_certificate_identity(
        mut self,
        client_certificate_identity: Option<ClientCertificateIdentity>,
    ) -> Self {
        self.client_certificate_identity = client_certificate_identity;
        self
    }

    /// 设置一次性内存秘密请求头 JSON。
    #[must_use]
    pub fn with_secret_custom_headers(
        mut self,
        secret_custom_headers: Option<SecretValue>,
    ) -> Self {
        self.secret_custom_headers = secret_custom_headers;
        self
    }
}

impl fmt::Debug for AzureOpenAiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureOpenAiConfig")
            .field("base_url", &"[REDACTED]")
            .field("deployment", &self.deployment)
            .field("api_version", &self.api_version)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("has_custom_headers", &self.custom_headers.is_some())
            .field(
                "has_secret_custom_headers",
                &self.secret_custom_headers.is_some(),
            )
            .field("has_proxy_url", &self.proxy_url.is_some())
            .field(
                "has_proxy_authentication",
                &self.proxy_authentication.is_some(),
            )
            .field("request_timeout", &self.request_timeout)
            .field("connection_timeout", &self.connection_timeout)
            .field("streaming_idle_timeout", &self.streaming_idle_timeout)
            .field(
                "has_trusted_certificates_pem",
                &self.trusted_certificates_pem.is_some(),
            )
            .field(
                "has_client_certificate_identity",
                &self.client_certificate_identity.is_some(),
            )
            .finish()
    }
}

impl OpenAiConfig {
    /// 创建没有凭据的本地或测试配置。
    #[must_use]
    pub fn without_credential(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credential: None,
            organization: None,
            project: None,
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 创建携带一次性内存凭据的配置。
    #[must_use]
    pub fn with_credential(base_url: impl Into<String>, credential: SecretValue) -> Self {
        Self {
            base_url: base_url.into(),
            credential: Some(credential),
            organization: None,
            project: None,
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 设置可选的非秘密组织标识。
    #[must_use]
    pub fn with_organization(mut self, organization: Option<String>) -> Self {
        self.organization = organization;
        self
    }

    /// 设置不含凭据的代理地址。
    #[must_use]
    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }

    /// 设置一次性内存代理认证。
    #[must_use]
    pub fn with_proxy_authentication(mut self, proxy_authentication: Option<SecretValue>) -> Self {
        self.proxy_authentication = proxy_authentication;
        self
    }

    /// 设置请求总超时。
    #[must_use]
    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// 设置连接建立超时。
    #[must_use]
    pub fn with_connection_timeout(mut self, connection_timeout: Duration) -> Self {
        self.connection_timeout = connection_timeout;
        self
    }

    /// 设置流式响应空闲超时。
    #[must_use]
    pub fn with_streaming_idle_timeout(mut self, streaming_idle_timeout: Duration) -> Self {
        self.streaming_idle_timeout = streaming_idle_timeout;
        self
    }

    /// 设置自定义可信证书 PEM；系统证书仍然保留。
    #[must_use]
    pub fn with_trusted_certificates_pem(
        mut self,
        trusted_certificates_pem: Option<String>,
    ) -> Self {
        self.trusted_certificates_pem = trusted_certificates_pem;
        self
    }

    /// 设置一次性内存 TLS 客户端证书身份。
    #[must_use]
    pub fn with_client_certificate_identity(
        mut self,
        client_certificate_identity: Option<ClientCertificateIdentity>,
    ) -> Self {
        self.client_certificate_identity = client_certificate_identity;
        self
    }

    /// 设置可选的非秘密项目标识。
    #[must_use]
    pub fn with_project(mut self, project: Option<String>) -> Self {
        self.project = project;
        self
    }

    /// 设置受限非秘密请求头 JSON。
    #[must_use]
    pub fn with_custom_headers(mut self, custom_headers: Option<String>) -> Self {
        self.custom_headers = custom_headers;
        self
    }

    /// 设置一次性内存秘密请求头 JSON。
    #[must_use]
    pub fn with_secret_custom_headers(
        mut self,
        secret_custom_headers: Option<SecretValue>,
    ) -> Self {
        self.secret_custom_headers = secret_custom_headers;
        self
    }
}

impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiConfig")
            .field("base_url", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("has_organization", &self.organization.is_some())
            .field("has_project", &self.project.is_some())
            .field("has_custom_headers", &self.custom_headers.is_some())
            .field(
                "has_secret_custom_headers",
                &self.secret_custom_headers.is_some(),
            )
            .field("has_proxy_url", &self.proxy_url.is_some())
            .field(
                "has_proxy_authentication",
                &self.proxy_authentication.is_some(),
            )
            .field("request_timeout", &self.request_timeout)
            .field("connection_timeout", &self.connection_timeout)
            .field("streaming_idle_timeout", &self.streaming_idle_timeout)
            .field(
                "has_trusted_certificates_pem",
                &self.trusted_certificates_pem.is_some(),
            )
            .field(
                "has_client_certificate_identity",
                &self.client_certificate_identity.is_some(),
            )
            .finish()
    }
}

/// 配置 `OpenAI` Responses API 端点。
pub struct OpenAiResponsesConfig {
    /// 通常以 `/v1/` 结尾的基础地址。
    pub base_url: String,
    /// 可选的一次性内存凭据。
    pub credential: Option<SecretValue>,
    /// 可选的非秘密组织标识。
    pub organization: Option<String>,
    /// 可选的非秘密项目标识。
    pub project: Option<String>,
    /// 可选的受限非秘密请求头 JSON。
    pub custom_headers: Option<String>,
    /// 可选的一次性内存秘密请求头 JSON。
    pub secret_custom_headers: Option<SecretValue>,
    /// 可选的不含凭据代理地址。
    pub proxy_url: Option<String>,
    /// 可选的一次性内存代理认证。
    pub proxy_authentication: Option<SecretValue>,
    /// 连接和普通响应超时。
    pub request_timeout: Duration,
    /// 建立网络连接的超时。
    pub connection_timeout: Duration,
    /// 流式响应等待下一数据块的超时。
    pub streaming_idle_timeout: Duration,
    /// 可选的自定义可信证书 PEM；不会关闭 TLS 校验。
    pub trusted_certificates_pem: Option<String>,
    /// 可选的一次性内存 TLS 客户端证书身份。
    pub client_certificate_identity: Option<ClientCertificateIdentity>,
}

impl OpenAiResponsesConfig {
    /// 创建没有凭据的本地或测试配置。
    #[must_use]
    pub fn without_credential(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credential: None,
            organization: None,
            project: None,
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 创建携带一次性内存凭据的配置。
    #[must_use]
    pub fn with_credential(base_url: impl Into<String>, credential: SecretValue) -> Self {
        Self {
            base_url: base_url.into(),
            credential: Some(credential),
            organization: None,
            project: None,
            custom_headers: None,
            secret_custom_headers: None,
            proxy_url: None,
            proxy_authentication: None,
            request_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
            streaming_idle_timeout: Duration::from_secs(60),
            trusted_certificates_pem: None,
            client_certificate_identity: None,
        }
    }

    /// 设置可选的非秘密组织标识。
    #[must_use]
    pub fn with_organization(mut self, organization: Option<String>) -> Self {
        self.organization = organization;
        self
    }

    /// 设置不含凭据的代理地址。
    #[must_use]
    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }

    /// 设置一次性内存代理认证。
    #[must_use]
    pub fn with_proxy_authentication(mut self, proxy_authentication: Option<SecretValue>) -> Self {
        self.proxy_authentication = proxy_authentication;
        self
    }

    /// 设置请求总超时。
    #[must_use]
    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// 设置连接建立超时。
    #[must_use]
    pub fn with_connection_timeout(mut self, connection_timeout: Duration) -> Self {
        self.connection_timeout = connection_timeout;
        self
    }

    /// 设置流式响应空闲超时。
    #[must_use]
    pub fn with_streaming_idle_timeout(mut self, streaming_idle_timeout: Duration) -> Self {
        self.streaming_idle_timeout = streaming_idle_timeout;
        self
    }

    /// 设置自定义可信证书 PEM；系统证书仍然保留。
    #[must_use]
    pub fn with_trusted_certificates_pem(
        mut self,
        trusted_certificates_pem: Option<String>,
    ) -> Self {
        self.trusted_certificates_pem = trusted_certificates_pem;
        self
    }

    /// 设置一次性内存 TLS 客户端证书身份。
    #[must_use]
    pub fn with_client_certificate_identity(
        mut self,
        client_certificate_identity: Option<ClientCertificateIdentity>,
    ) -> Self {
        self.client_certificate_identity = client_certificate_identity;
        self
    }

    /// 设置可选的非秘密项目标识。
    #[must_use]
    pub fn with_project(mut self, project: Option<String>) -> Self {
        self.project = project;
        self
    }

    /// 设置受限非秘密请求头 JSON。
    #[must_use]
    pub fn with_custom_headers(mut self, custom_headers: Option<String>) -> Self {
        self.custom_headers = custom_headers;
        self
    }

    /// 设置一次性内存秘密请求头 JSON。
    #[must_use]
    pub fn with_secret_custom_headers(
        mut self,
        secret_custom_headers: Option<SecretValue>,
    ) -> Self {
        self.secret_custom_headers = secret_custom_headers;
        self
    }
}

impl fmt::Debug for OpenAiResponsesConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesConfig")
            .field("base_url", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("has_organization", &self.organization.is_some())
            .field("has_project", &self.project.is_some())
            .field("has_custom_headers", &self.custom_headers.is_some())
            .field(
                "has_secret_custom_headers",
                &self.secret_custom_headers.is_some(),
            )
            .field("has_proxy_url", &self.proxy_url.is_some())
            .field(
                "has_proxy_authentication",
                &self.proxy_authentication.is_some(),
            )
            .field("request_timeout", &self.request_timeout)
            .field("connection_timeout", &self.connection_timeout)
            .field("streaming_idle_timeout", &self.streaming_idle_timeout)
            .field(
                "has_trusted_certificates_pem",
                &self.trusted_certificates_pem.is_some(),
            )
            .field(
                "has_client_certificate_identity",
                &self.client_certificate_identity.is_some(),
            )
            .finish()
    }
}

/// 实现模型发现和 Chat Completions 流。
#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    client: Client,
    base_url: Url,
    credential: Arc<Mutex<CredentialState>>,
    organization: Option<String>,
    project: Option<String>,
    custom_headers: Vec<(HeaderName, HeaderValue)>,
    session_cancellation: CancellationToken,
    streaming_idle_timeout: Duration,
    protocol: OpenAiProtocol,
}

#[derive(Clone, Debug)]
enum OpenAiProtocol {
    ChatCompletions,
    Responses,
    AzureChatCompletions {
        deployment: String,
        api_version: String,
    },
}

enum CredentialState {
    NotRequired,
    Available(SecretValue),
    Cleared,
}

impl fmt::Debug for OpenAiCompatibleProvider {
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
            .debug_struct("OpenAiCompatibleProvider")
            .field("base_url", &"[REDACTED]")
            .field("credential_state", &credential_state)
            .field("session_closed", &self.session_cancellation.is_cancelled())
            .field("streaming_idle_timeout", &self.streaming_idle_timeout)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}

impl OpenAiCompatibleProvider {
    /// 在请求宿主秘密之前验证不含秘密的端点策略。
    pub fn validate_endpoint(base_url: &str) -> Result<(), TranslationError> {
        validated_base_url(base_url).map(|_| ())
    }

    /// 创建拒绝跨源重定向的适配器。
    pub fn new(config: OpenAiConfig) -> Result<Self, TranslationError> {
        Self::new_with_protocol(
            &config.base_url,
            config.credential,
            config.organization,
            config.project,
            config.custom_headers.as_deref(),
            config.secret_custom_headers.as_ref(),
            config.proxy_url.as_deref(),
            config.proxy_authentication.as_ref(),
            config.request_timeout,
            config.connection_timeout,
            config.streaming_idle_timeout,
            config.trusted_certificates_pem.as_deref(),
            config.client_certificate_identity.as_ref(),
            OpenAiProtocol::ChatCompletions,
        )
    }

    /// 创建使用 typed SSE 事件的 `OpenAI` Responses API 适配器。
    pub fn new_responses(config: OpenAiResponsesConfig) -> Result<Self, TranslationError> {
        Self::new_with_protocol(
            &config.base_url,
            config.credential,
            config.organization,
            config.project,
            config.custom_headers.as_deref(),
            config.secret_custom_headers.as_ref(),
            config.proxy_url.as_deref(),
            config.proxy_authentication.as_ref(),
            config.request_timeout,
            config.connection_timeout,
            config.streaming_idle_timeout,
            config.trusted_certificates_pem.as_deref(),
            config.client_certificate_identity.as_ref(),
            OpenAiProtocol::Responses,
        )
    }

    /// 创建 Azure `OpenAI` Chat Completions 适配器。
    pub fn new_azure(config: AzureOpenAiConfig) -> Result<Self, TranslationError> {
        let deployment = validate_segment(&config.deployment, "Azure deployment")?;
        let api_version = validate_query_value(&config.api_version, "Azure API version")?;
        let resource_url = validated_base_url(&config.base_url)?;
        let base_url = resource_url
            .join("openai/deployments/")
            .and_then(|url| url.join(&format!("{deployment}/")))
            .map_err(|_| {
                TranslationError::new(
                    ErrorKind::InvalidEndpoint,
                    "Azure OpenAI deployment endpoint is invalid.",
                )
            })?;
        Self::new_with_protocol(
            base_url.as_str(),
            config.credential,
            None,
            None,
            config.custom_headers.as_deref(),
            config.secret_custom_headers.as_ref(),
            config.proxy_url.as_deref(),
            config.proxy_authentication.as_ref(),
            config.request_timeout,
            config.connection_timeout,
            config.streaming_idle_timeout,
            config.trusted_certificates_pem.as_deref(),
            config.client_certificate_identity.as_ref(),
            OpenAiProtocol::AzureChatCompletions {
                deployment,
                api_version,
            },
        )
    }

    /// 在请求宿主秘密之前验证 Azure 资源端点和部署配置。
    pub fn validate_azure_endpoint(
        base_url: &str,
        deployment: &str,
        api_version: &str,
    ) -> Result<(), TranslationError> {
        let config = AzureOpenAiConfig::without_credential(base_url, deployment, api_version);
        Self::new_azure(config).map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_protocol(
        base_url: &str,
        credential: Option<SecretValue>,
        organization: Option<String>,
        project: Option<String>,
        custom_headers: Option<&str>,
        secret_custom_headers: Option<&SecretValue>,
        proxy_url: Option<&str>,
        proxy_authentication: Option<&SecretValue>,
        request_timeout: Duration,
        connection_timeout: Duration,
        streaming_idle_timeout: Duration,
        trusted_certificates_pem: Option<&str>,
        client_certificate_identity: Option<&ClientCertificateIdentity>,
        protocol: OpenAiProtocol,
    ) -> Result<Self, TranslationError> {
        let base_url = validated_base_url(base_url)?;
        let mut custom_headers = parse_custom_headers(custom_headers, false)?;
        custom_headers.extend(parse_custom_headers(
            secret_custom_headers.map(SecretValue::expose_secret),
            true,
        )?);
        if custom_headers.len() > MAX_CUSTOM_HEADERS {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider custom headers are invalid.",
            ));
        }
        let mut client_builder = Client::builder()
            .redirect(Policy::none())
            .timeout(request_timeout)
            .connect_timeout(connection_timeout);
        if let Some(proxy_url) = proxy_url {
            let mut proxy =
                reqwest::Proxy::all(proxy_url).map_err(|error| map_reqwest_error(&error))?;
            if let Some(secret) = proxy_authentication {
                let credentials = ProxyAuthentication::parse(secret)?;
                proxy = proxy.basic_auth(credentials.username(), credentials.password());
            }
            client_builder = client_builder.proxy(proxy);
        } else if proxy_authentication.is_some() {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Proxy credentials require a proxy URL.",
            ));
        }
        if let Some(pem) = trusted_certificates_pem {
            let certificates =
                reqwest::Certificate::from_pem_bundle(pem.as_bytes()).map_err(|_| {
                    TranslationError::new(
                        ErrorKind::InvalidConfiguration,
                        "Provider trusted certificates are invalid.",
                    )
                })?;
            for certificate in certificates {
                client_builder = client_builder.add_root_certificate(certificate);
            }
        }
        if let Some(identity) = client_certificate_identity {
            let identity = reqwest::Identity::from_pem(identity.expose_secret().as_bytes())
                .map_err(|_| {
                    TranslationError::new(
                        ErrorKind::InvalidConfiguration,
                        "Provider client certificate identity is invalid.",
                    )
                })?;
            client_builder = client_builder.identity(identity);
        }
        let client = client_builder
            .build()
            .map_err(|error| map_reqwest_error(&error))?;
        Ok(Self {
            client,
            base_url,
            credential: Arc::new(Mutex::new(
                credential.map_or(CredentialState::NotRequired, CredentialState::Available),
            )),
            organization,
            project,
            custom_headers,
            session_cancellation: CancellationToken::new(),
            streaming_idle_timeout,
            protocol,
        })
    }

    /// 取消该会话的请求并立即清除所有共享引用使用的凭据。
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
        if self.session_cancellation.is_cancelled() {
            return Err(TranslationError::cancelled());
        }
        let mut headers = HeaderMap::new();
        for (name, value) in &self.custom_headers {
            headers.append(name.clone(), value.clone());
        }
        let request = request.headers(headers);
        let request = if matches!(
            &self.protocol,
            OpenAiProtocol::ChatCompletions | OpenAiProtocol::Responses
        ) {
            if let Some(organization) = self.organization.as_deref() {
                request.header("OpenAI-Organization", organization)
            } else {
                request
            }
        } else {
            request
        };
        let request = if matches!(
            &self.protocol,
            OpenAiProtocol::ChatCompletions | OpenAiProtocol::Responses
        ) {
            if let Some(project) = self.project.as_deref() {
                request.header("OpenAI-Project", project)
            } else {
                request
            }
        } else {
            request
        };
        match &mut *credential {
            CredentialState::NotRequired => Ok(request),
            CredentialState::Available(secret) => match self.protocol {
                OpenAiProtocol::ChatCompletions | OpenAiProtocol::Responses => {
                    Ok(request.bearer_auth(secret.expose_secret()))
                }
                OpenAiProtocol::AzureChatCompletions { .. } => {
                    Ok(request.header("api-key", secret.expose_secret()))
                }
            },
            CredentialState::Cleared => Err(TranslationError::new(
                ErrorKind::SecretUnavailable,
                "The provider credential session was cleared.",
            )),
        }
    }

    fn endpoint(&self, path: &str) -> Result<Url, TranslationError> {
        let mut endpoint = self.base_url.join(path).map_err(|_| {
            TranslationError::new(ErrorKind::InvalidEndpoint, "Provider endpoint is invalid.")
        })?;
        if let OpenAiProtocol::AzureChatCompletions {
            ref api_version, ..
        } = self.protocol
        {
            endpoint
                .query_pairs_mut()
                .append_pair("api-version", api_version);
        }
        Ok(endpoint)
    }

    #[allow(clippy::too_many_lines)]
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
        let responses_protocol = matches!(self.protocol, OpenAiProtocol::Responses);
        let body = if responses_protocol {
            ProviderRequestBody::Responses(ResponsesRequest {
                model: request.model_id,
                stream: true,
                input: vec![
                    ResponsesInput {
                        role: "developer",
                        content: translation_prompt(
                            request.source_locale.as_deref(),
                            &request.target_locale,
                            request.quality_mode,
                            Some(&request.preset),
                            &marker_instruction,
                        ),
                    },
                    ResponsesInput {
                        role: "user",
                        content: format!("<source>\n{}\n</source>", request.source_text),
                    },
                ],
            })
        } else {
            ProviderRequestBody::Chat(ChatRequest {
                model: request.model_id,
                stream: true,
                stream_options: matches!(self.protocol, OpenAiProtocol::ChatCompletions).then_some(
                    StreamOptions {
                        include_usage: true,
                    },
                ),
                messages: vec![
                    ChatMessage {
                        role: "system",
                        content: translation_prompt(
                            request.source_locale.as_deref(),
                            &request.target_locale,
                            request.quality_mode,
                            Some(&request.preset),
                            &marker_instruction,
                        ),
                    },
                    ChatMessage {
                        role: "user",
                        content: format!("<source>\n{}\n</source>", request.source_text),
                    },
                ],
            })
        };
        let endpoint = if responses_protocol {
            "responses"
        } else {
            "chat/completions"
        };
        let request = self
            .request(self.client.post(self.endpoint(endpoint)?))?
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
        let streaming_idle_timeout = self.streaming_idle_timeout;
        let stream = try_stream! {
            let mut chat_decoder = SseDecoder::default();
            let mut responses_decoder = ResponsesSseDecoder::default();
            let mut total_bytes = 0usize;
            let mut completed = false;
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => Err(TranslationError::cancelled()),
                    () = session_cancellation.cancelled() => Err(TranslationError::cancelled()),
                    item = tokio::time::timeout(streaming_idle_timeout, bytes.next()) => match item {
                        Ok(item) => Ok(item),
                        Err(_) => Err(TranslationError::new(
                            ErrorKind::Timeout,
                            "Provider stream timed out waiting for the next chunk.",
                        )),
                    },
                }?;
                let Some(chunk) = next else {
                    break;
                };
                let chunk = chunk.map_err(|error| map_reqwest_error(&error))?;
                total_bytes = total_bytes.saturating_add(chunk.len());
                if total_bytes > MAX_RESPONSE_BYTES {
                    Err(TranslationError::new(
                        ErrorKind::MalformedResponse,
                        "Provider stream exceeded the response-size limit.",
                    ))?;
                }
                let messages = if responses_protocol {
                    responses_decoder.push(&chunk)?
                } else {
                    chat_decoder.push(&chunk)?
                };
                for message in messages {
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
                        SseMessage::UsageAndDone(usage) => {
                            yield TranslationStreamEvent::Usage(usage);
                            completed = true;
                            break;
                        }
                        SseMessage::Done => {
                            completed = true;
                            break;
                        }
                    }
                }
                if completed {
                    break;
                }
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

fn parse_custom_headers(
    custom_headers: Option<&str>,
    allow_secret_values: bool,
) -> Result<Vec<(HeaderName, HeaderValue)>, TranslationError> {
    let Some(custom_headers) = custom_headers.filter(|value| !value.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let headers =
        serde_json::from_str::<std::collections::BTreeMap<String, String>>(custom_headers)
            .map_err(|_| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Provider custom headers are invalid.",
                )
            })?;
    if headers.is_empty() || headers.len() > MAX_CUSTOM_HEADERS {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "Provider custom headers are invalid.",
        ));
    }
    headers
        .into_iter()
        .map(|(name, value)| {
            if name.is_empty()
                || name.len() > MAX_CUSTOM_HEADER_NAME_BYTES
                || is_forbidden_custom_header_name(&name)
                || value.is_empty()
                || value.len() > MAX_CUSTOM_HEADER_VALUE_BYTES
                || value.chars().any(char::is_control)
                || (!allow_secret_values && looks_like_credential(&value))
            {
                return Err(TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Provider custom headers are invalid.",
                ));
            }
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Provider custom header name is invalid.",
                )
            })?;
            let value = HeaderValue::from_str(&value).map_err(|_| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "Provider custom header value is invalid.",
                )
            })?;
            Ok((name, value))
        })
        .collect()
}

// 拒绝可能覆盖内置请求元数据或承载凭据的请求头名称。
fn is_forbidden_custom_header_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "api-key",
        "x-api-key",
        "openai-organization",
        "openai-project",
        "content-type",
        "accept",
        "user-agent",
        "secret",
        "token",
        "credential",
    ]
    .iter()
    .any(|forbidden| name == *forbidden || name.contains(forbidden))
}

// 拒绝常见 API 凭据形态，避免非秘密字段承载秘密。
fn looks_like_credential(value: &str) -> bool {
    let value = value.trim();
    value.contains("PRIVATE KEY-----")
        || value.contains(concat!("github_", "pat_"))
        || value.to_ascii_lowercase().starts_with("bearer ")
        || value
            .match_indices("sk-")
            .any(|(start, _)| value.len().saturating_sub(start + 3) >= 20)
        || value
            .match_indices("ghp_")
            .any(|(start, _)| value.len().saturating_sub(start + 4) >= 24)
}

// 验证 Azure 路径段，避免把路径控制字符或额外层级带入请求地址。
fn validate_segment(value: &str, label: &str) -> Result<String, TranslationError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            format!("{label} is invalid."),
        ));
    }
    Ok(value.to_owned())
}

// 验证 Azure 查询值，拒绝凭据和 URL 控制字符进入配置。
fn validate_query_value(value: &str, label: &str) -> Result<String, TranslationError> {
    if value.is_empty() || value.chars().any(char::is_control) || value.contains('&') {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            format!("{label} is invalid."),
        ));
    }
    Ok(value.to_owned())
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn list_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError> {
        if let OpenAiProtocol::AzureChatCompletions { ref deployment, .. } = self.protocol {
            return Ok(vec![ModelDescriptor {
                id: deployment.clone(),
                display_name: deployment.clone(),
                source: ModelSource::Manual,
            }]);
        }
        let request = self
            .request(self.client.get(self.endpoint("models")?))?
            .send();
        let response = tokio::select! {
            biased;
            () = self.session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            response = request => response.map_err(|error| map_reqwest_error(&error))?,
        };
        let response = ensure_success(response)?;
        let body_request = response.json();
        let body: ModelListResponse = tokio::select! {
            biased;
            () = self.session_cancellation.cancelled() => return Err(TranslationError::cancelled()),
            body = body_request => body.map_err(|error| map_reqwest_error(&error))?,
        };
        Ok(body
            .data
            .into_iter()
            .map(|model| ModelDescriptor {
                display_name: model.id.clone(),
                id: model.id,
                source: ModelSource::Discovered,
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
            while let Some(delta) = chunk_stream.next().await {
                yield delta?;
            }
            for chunk in chunks {
                let mut chunk_request = request.clone();
                chunk_request.source_text = chunk.text().to_owned();
                let mut chunk_stream = provider
                    .translate_protected_stream(chunk_request, chunk, cancellation.clone())
                    .await?;
                while let Some(delta) = chunk_stream.next().await {
                    yield delta?;
                }
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
#[serde(untagged)]
enum ProviderRequestBody {
    Chat(ChatRequest),
    Responses(ResponsesRequest),
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    stream: bool,
    input: Vec<ResponsesInput>,
}

#[derive(Serialize)]
struct ResponsesInput {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<ModelResponse>,
}

#[derive(Deserialize)]
struct ModelResponse {
    id: String,
}

#[derive(Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<UsageResponse>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

enum SseMessage {
    Delta(String),
    Usage(UsageRecord),
    UsageAndDone(UsageRecord),
    Done,
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)]
struct UsageResponse {
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: Option<u64>,
    #[serde(default, alias = "completion_tokens")]
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
    if data == "[DONE]" {
        return Ok(Some(SseMessage::Done));
    }
    let response: StreamResponse = serde_json::from_str(&data).map_err(|_| {
        TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider stream contained malformed JSON.",
        )
    })?;
    if let Some(usage) = response.usage {
        return Ok(Some(SseMessage::Usage(usage.into_record())));
    }
    let text = response
        .choices
        .first()
        .and_then(|choice| choice.delta.content.clone())
        .unwrap_or_default();
    Ok(Some(SseMessage::Delta(text)))
}

#[derive(Default)]
struct ResponsesSseDecoder {
    buffer: Vec<u8>,
}

impl ResponsesSseDecoder {
    fn push(&mut self, chunk: &Bytes) -> Result<Vec<SseMessage>, TranslationError> {
        self.buffer.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some((position, delimiter_len)) = find_event_boundary(&self.buffer) {
            let event = self.buffer.drain(..position).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            if let Some(message) = parse_responses_event(&event)? {
                output.push(message);
            }
        }
        Ok(output)
    }
}

fn parse_responses_event(event: &[u8]) -> Result<Option<SseMessage>, TranslationError> {
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
    let response: ResponsesStreamEvent = serde_json::from_str(&data).map_err(|_| {
        TranslationError::new(
            ErrorKind::MalformedResponse,
            "Provider Responses stream contained malformed JSON.",
        )
    })?;
    match response.event_type.as_str() {
        "response.output_text.delta" => {
            Ok(Some(SseMessage::Delta(response.delta.unwrap_or_default())))
        }
        "response.completed" => {
            if let Some(usage) = response.response.and_then(|response| response.usage) {
                return Ok(Some(SseMessage::UsageAndDone(usage.into_record())));
            }
            Ok(Some(SseMessage::Done))
        }
        "error" | "response.failed" => Err(TranslationError::new(
            ErrorKind::Network,
            "Provider Responses stream reported an error.",
        )),
        _ => Ok(None),
    }
}

#[derive(Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<String>,
    #[serde(default)]
    response: Option<ResponsesCompleted>,
}

#[derive(Deserialize)]
struct ResponsesCompleted {
    #[serde(default)]
    usage: Option<UsageResponse>,
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
    let kind = error_kind_for_http_status(status.as_u16());
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
    use super::{
        AzureOpenAiConfig, OpenAiCompatibleProvider, OpenAiConfig, OpenAiResponsesConfig,
        ResponsesSseDecoder, SseDecoder, SseMessage,
    };
    use bytes::Bytes;
    use futures_util::StreamExt;
    use linguamesh_domain::{
        ClientCertificateIdentity, ErrorKind, Glossary, GlossaryEntry, SecretValue,
        TranslationRequest,
    };
    use linguamesh_provider_api::ModelProvider;
    use std::fmt::Write;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn invalid_trusted_certificates_are_rejected_without_disabling_tls() {
        let error = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("https://provider.example/v1/")
                .with_trusted_certificates_pem(Some(
                    "-----BEGIN CERTIFICATE-----\nnot-a-certificate\n-----END CERTIFICATE-----"
                        .to_owned(),
                )),
        )
        .expect_err("invalid certificate bundle should fail configuration");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert_eq!(error.message, "Provider trusted certificates are invalid.");
    }

    #[test]
    fn invalid_client_certificate_identity_is_rejected_without_disabling_tls() {
        let identity = ClientCertificateIdentity::parse(&SecretValue::new(concat!(
            "-----BEGIN CERTIFICATE-----\nnot-a-certificate\n-----END CERTIFICATE-----\n",
            "-----BEGIN ",
            "PRIVATE KEY-----\nnot-a-key\n-----END ",
            "PRIVATE KEY-----"
        )))
        .expect("bounded identity markers");
        let error = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("https://provider.example/v1/")
                .with_client_certificate_identity(Some(identity)),
        )
        .expect_err("invalid identity should fail configuration");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert_eq!(
            error.message,
            "Provider client certificate identity is invalid."
        );
    }

    #[test]
    fn diagnostics_redact_endpoint_and_credential() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_PROVIDER_DEBUG_SECRET_1234567890");
        const ENDPOINT: &str = "https://provider.example/v1/";
        let config = OpenAiConfig::with_credential(ENDPOINT, SecretValue::new(SECRET_CANARY));
        let config_debug = format!("{config:?}");
        assert!(!config_debug.contains(SECRET_CANARY));
        assert!(!config_debug.contains(ENDPOINT));

        let provider = OpenAiCompatibleProvider::new(config).expect("provider");
        let provider_debug = format!("{provider:?}");
        assert!(!provider_debug.contains(SECRET_CANARY));
        assert!(!provider_debug.contains(ENDPOINT));
    }

    #[test]
    fn decoder_handles_fragmented_utf8_and_lines() {
        let payload =
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: [DONE]\n\n";
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
    fn decoder_extracts_chat_usage_before_done() {
        let payload = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut decoder = SseDecoder::default();
        let messages = decoder
            .push(&Bytes::from_static(payload.as_bytes()))
            .expect("usage events");
        assert!(matches!(&messages[0], SseMessage::Delta(text) if text == "你好"));
        assert!(matches!(
            &messages[1],
            SseMessage::Usage(record)
                if record.input_tokens == Some(4)
                    && record.output_tokens == Some(2)
                    && record.total_tokens == Some(6)
        ));
        assert!(matches!(messages[2], SseMessage::Done));
    }

    #[test]
    fn responses_decoder_handles_typed_events_and_fragmented_utf8() {
        let payload = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\"}\n\n",
        );
        let bytes = payload.as_bytes();
        let split = payload.find('好').expect("unicode split") + 1;
        let mut decoder = ResponsesSseDecoder::default();
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
    fn responses_decoder_extracts_completion_usage() {
        let event = concat!(
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":7,\"output_tokens\":3,\"total_tokens\":10}}}\n\n",
        );
        let mut decoder = ResponsesSseDecoder::default();
        let messages = decoder
            .push(&Bytes::from_static(event.as_bytes()))
            .expect("usage event");
        assert!(matches!(
            &messages[0],
            SseMessage::UsageAndDone(record)
                if record.input_tokens == Some(7)
                    && record.output_tokens == Some(3)
                    && record.total_tokens == Some(10)
        ));
    }

    #[test]
    fn responses_diagnostics_redact_endpoint_and_credential() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_RESPONSES_DEBUG_SECRET_1234567890");
        const ENDPOINT: &str = "https://provider.example/v1/";
        let config =
            OpenAiResponsesConfig::with_credential(ENDPOINT, SecretValue::new(SECRET_CANARY));
        let config_debug = format!("{config:?}");
        assert!(!config_debug.contains(SECRET_CANARY));
        assert!(!config_debug.contains(ENDPOINT));
        let provider = OpenAiCompatibleProvider::new_responses(config).expect("provider");
        let provider_debug = format!("{provider:?}");
        assert!(!provider_debug.contains(SECRET_CANARY));
        assert!(!provider_debug.contains(ENDPOINT));
    }

    #[test]
    fn endpoint_policy_allows_https_and_loopback_http_only() {
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "https://provider.example/v1/"
            ))
            .is_ok()
        );
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "http://127.0.0.1:8080/v1/"
            ))
            .is_ok()
        );
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "http://provider.example/v1/"
            ))
            .is_err()
        );
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "https://user:secret@provider.example/v1/"
            ))
            .is_err()
        );
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "https://provider.example/v1/?api_key=secret"
            ))
            .is_err()
        );
        assert!(
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(
                "https://provider.example/v1/#fragment"
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn explicit_proxy_receives_provider_request() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("proxy listener");
        let proxy_address = listener.local_addr().expect("proxy address");
        let proxy_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("proxy connection");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = socket.read(&mut chunk).await.expect("proxy request");
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("GET http://127.0.0.1:9/v1/models HTTP/1.1"));
            assert!(request.lines().any(|line| {
                line.to_ascii_lowercase()
                    .starts_with("proxy-authorization: basic ")
            }));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"data\":[]}",
                )
                .await
                .expect("proxy response");
        });
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("http://127.0.0.1:9/v1/")
                .with_proxy_url(Some(format!("http://{proxy_address}")))
                .with_proxy_authentication(Some(SecretValue::new("proxy-user:proxy-secret"))),
        )
        .expect("provider");
        let models = tokio::time::timeout(Duration::from_secs(1), provider.list_models())
            .await
            .expect("proxy request timeout")
            .expect("proxy response");
        assert!(models.is_empty());
        proxy_task.await.expect("proxy task");
    }

    #[test]
    fn organization_is_added_only_to_openai_protocol_requests() {
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("https://provider.example/v1/")
                .with_organization(Some("org-local".to_owned())),
        )
        .expect("provider");
        let request = provider
            .request(provider.client.get("https://provider.example/v1/models"))
            .expect("request")
            .build()
            .expect("built request");
        assert_eq!(
            request
                .headers()
                .get("OpenAI-Organization")
                .and_then(|value| value.to_str().ok()),
            Some("org-local")
        );
    }

    #[test]
    fn project_is_added_only_to_openai_protocol_requests() {
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("https://provider.example/v1/")
                .with_project(Some("project-local".to_owned())),
        )
        .expect("provider");
        let request = provider
            .request(provider.client.get("https://provider.example/v1/models"))
            .expect("request")
            .build()
            .expect("built request");
        assert_eq!(
            request
                .headers()
                .get("OpenAI-Project")
                .and_then(|value| value.to_str().ok()),
            Some("project-local")
        );
    }

    #[test]
    fn custom_headers_are_added_without_replacing_auth_metadata() {
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::with_credential(
                "https://provider.example/v1/",
                SecretValue::new("test-secret"),
            )
            .with_custom_headers(Some(
                r#"{"X-Trace-Mode":"local","X-Feature":"draft"}"#.to_owned(),
            )),
        )
        .expect("provider");
        let request = provider
            .request(provider.client.get("https://provider.example/v1/models"))
            .expect("request")
            .build()
            .expect("built request");
        assert_eq!(
            request
                .headers()
                .get("X-Trace-Mode")
                .and_then(|value| value.to_str().ok()),
            Some("local")
        );
        assert!(request.headers().contains_key("authorization"));
    }

    #[test]
    fn secret_custom_headers_are_added_without_debug_leakage() {
        const SECRET_HEADER_VALUE: &str = "sensitive-header-value";
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential("https://provider.example/v1/")
                .with_secret_custom_headers(Some(SecretValue::new(format!(
                    r#"{{"X-Trace-Mode":"{SECRET_HEADER_VALUE}"}}"#
                )))),
        )
        .expect("provider");
        let request = provider
            .request(provider.client.get("https://provider.example/v1/models"))
            .expect("request")
            .build()
            .expect("built request");
        assert_eq!(
            request
                .headers()
                .get("X-Trace-Mode")
                .and_then(|value| value.to_str().ok()),
            Some(SECRET_HEADER_VALUE)
        );
        assert!(!format!("{provider:?}").contains(SECRET_HEADER_VALUE));
    }

    #[test]
    fn custom_headers_reject_credentials_and_reserved_metadata() {
        for custom_headers in [
            r#"{"Authorization":"not-a-secret"}"#,
            r#"{"OpenAI-Organization":"tenant"}"#,
            concat!("{\"X-Trace\":\"s", "k-live-secret-value-1234567890\"}"),
        ] {
            assert!(
                OpenAiCompatibleProvider::new(
                    OpenAiConfig::without_credential("https://provider.example/v1/")
                        .with_custom_headers(Some(custom_headers.to_owned())),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn azure_custom_headers_are_added_without_replacing_api_key() {
        let provider = OpenAiCompatibleProvider::new_azure(
            AzureOpenAiConfig::with_credential(
                "http://127.0.0.1:8080/",
                "fake-deployment",
                "2024-10-21",
                SecretValue::new("azure-test-key"),
            )
            .with_custom_headers(Some(r#"{"X-Trace-Mode":"azure"}"#.to_owned())),
        )
        .expect("provider");
        let request =
            provider
                .request(provider.client.get(
                    "http://127.0.0.1:8080/openai/deployments/fake-deployment/chat/completions",
                ))
                .expect("request")
                .build()
                .expect("built request");
        assert_eq!(
            request
                .headers()
                .get("X-Trace-Mode")
                .and_then(|value| value.to_str().ok()),
            Some("azure")
        );
        assert_eq!(
            request
                .headers()
                .get("api-key")
                .and_then(|value| value.to_str().ok()),
            Some("azure-test-key")
        );
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
        let provider = OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(format!(
            "http://{address}/v1/"
        )))
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
                TranslationRequest::new("Hello", "zh-CN", "fake-translator"),
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

    #[tokio::test]
    async fn streaming_idle_timeout_interrupts_stalled_body() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let stalled_server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("connection");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .await
                .expect("response headers");
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let provider = OpenAiCompatibleProvider::new(
            OpenAiConfig::without_credential(format!("http://{address}/v1/"))
                .with_streaming_idle_timeout(Duration::from_millis(25)),
        )
        .expect("provider");
        let mut stream = provider
            .translate_stream(
                TranslationRequest::new("Hello", "zh-CN", "fake-translator"),
                CancellationToken::new(),
            )
            .await
            .expect("stream");
        let error = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream timeout")
            .expect("stream error event")
            .expect_err("stalled body should time out");
        assert_eq!(error.kind, ErrorKind::Timeout);
        assert_eq!(
            error.message,
            "Provider stream timed out waiting for the next chunk."
        );
        stalled_server.abort();
        let _ = stalled_server.await;
    }

    #[tokio::test]
    async fn protected_markers_and_glossary_are_restored_across_stream_fragments() {
        let source = "Keep https://example.com/path and `git status` with LinguaMesh unchanged.";
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
            let content = body["messages"][1]["content"]
                .as_str()
                .expect("source content")
                .strip_prefix("<source>\n")
                .and_then(|value| value.strip_suffix("\n</source>"))
                .expect("source delimiters")
                .to_owned();
            assert!(!content.contains("https://example.com/path"));
            assert!(!content.contains("with LinguaMesh"));
            let marker_start = content.find("__LINGUAMESH_PROTECTED_").expect("marker");
            let split = marker_start + 7;
            let fragments = [&content[..split], &content[split..]];
            let mut events = String::new();
            for fragment in fragments {
                let data = serde_json::json!({
                    "choices": [{"delta": {"content": fragment}}]
                });
                writeln!(&mut events, "data: {data}").expect("event");
                events.push('\n');
            }
            events.push_str("data: [DONE]\n\n");
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

        let provider = OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(format!(
            "http://{address}/v1/"
        )))
        .expect("provider");
        let mut stream = provider
            .translate_stream(
                TranslationRequest::new(source, "zh-CN", "protected-translator").with_glossary(
                    Glossary::new(vec![
                        GlossaryEntry::new("LinguaMesh", "凌瓦网")
                            .expect("glossary entry")
                            .with_target_locale("zh-CN"),
                    ])
                    .expect("glossary"),
                ),
                CancellationToken::new(),
            )
            .await
            .expect("stream");
        let mut output = String::new();
        while let Some(delta) = stream.next().await {
            if let linguamesh_provider_api::TranslationStreamEvent::Text(text) =
                delta.expect("protected output")
            {
                output.push_str(&text);
            }
        }
        assert_eq!(
            output,
            "Keep https://example.com/path and `git status` with 凌瓦网 unchanged."
        );
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn long_text_is_chunked_in_order_and_each_chunk_streams() {
        let server = linguamesh_testkit::FakeProviderServer::start()
            .await
            .expect("server");
        let requests = server.chat_request_counter();
        let provider =
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(server.base_url()))
                .expect("provider");
        let request = TranslationRequest::new(
            "First sentence. Second sentence. Third sentence.",
            "zh-CN",
            "fake-translator",
        )
        .with_max_chunk_bytes(20);
        let mut stream = provider
            .translate_stream(request, CancellationToken::new())
            .await
            .expect("stream");
        let mut output = String::new();
        while let Some(delta) = stream.next().await {
            if let linguamesh_provider_api::TranslationStreamEvent::Text(text) =
                delta.expect("chunk output")
            {
                output.push_str(&text);
            }
        }
        let request_count = requests.load(Ordering::SeqCst);
        assert!(request_count > 1);
        assert_eq!(output, "你好，LinguaMesh！".repeat(request_count));
        server.shutdown().await;
    }
}
