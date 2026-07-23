#![doc = "`LinguaMesh` 的稳定领域类型。"]

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use url::{Host, Url};
use uuid::{Uuid, Variant, Version};

mod file_lease;
mod retry;
mod routing;

pub use file_lease::{FileLease, FileLeaseError, FileLeaseGuard, FileLeaseResource};
pub use retry::{
    DEFAULT_RETRY_BASE_DELAY_MS, DEFAULT_RETRY_CIRCUIT_COOLDOWN_MS,
    DEFAULT_RETRY_CIRCUIT_FAILURE_THRESHOLD, DEFAULT_RETRY_JITTER_PERCENT,
    DEFAULT_RETRY_MAX_BACKOFF_MS, RetryPolicy, RetryPolicyError,
};
pub use routing::{
    MAX_ROUTING_CANDIDATES, MAX_ROUTING_IDENTIFIER_BYTES, MAX_ROUTING_PROFILE_JSON_BYTES,
    RoutingCandidate, RoutingConstraints, RoutingContext, RoutingDecision, RoutingError,
    RoutingMode, RoutingPreference, RoutingProfile, RoutingRank, RoutingRejection,
    RoutingRejectionReason, deserialize_routing_profile, serialize_routing_profile,
};

/// 标识一次翻译操作。
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct OperationId(String);

impl OperationId {
    /// 创建不可预测的新操作标识。
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// 从已有稳定值创建标识。
    #[must_use]
    pub fn from_value(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 返回协议使用的字符串值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

/// 关联跨层事件和诊断。
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct CorrelationId(String);

impl CorrelationId {
    /// 创建不可预测的新关联标识。
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// 从已有稳定值创建标识。
    #[must_use]
    pub fn from_value(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 返回协议使用的字符串值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

/// 标识一次可关联且不可重放的宿主服务请求。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HostRequestId(String);

impl HostRequestId {
    /// 创建不可预测的新宿主请求标识。
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// 返回诊断和响应关联使用的稳定值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for HostRequestId {
    fn default() -> Self {
        Self::new()
    }
}

/// 描述模型条目的可信来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// 从提供商接口发现。
    Discovered,
    /// 从版本化目录加载。
    Catalog,
    /// 由用户明确输入。
    Manual,
}

/// 描述可选择的提供商模型。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// 提供商使用的稳定模型标识。
    pub id: String,
    /// 界面显示名称。
    pub display_name: String,
    /// 模型条目的来源。
    pub source: ModelSource,
}

const MAX_STABLE_ID_BYTES: usize = 128;
const MAX_PROFILE_TEXT_BYTES: usize = 2048;
const MAX_CUSTOM_HEADERS: usize = 16;
const MAX_CUSTOM_HEADER_NAME_BYTES: usize = 128;
const MAX_CUSTOM_HEADER_VALUE_BYTES: usize = 2048;
const MAX_PROXY_URL_BYTES: usize = 2048;

/// 标识一个持久化提供商配置。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProviderProfileId(String);

impl ProviderProfileId {
    /// 解析受限的稳定配置标识。
    pub fn parse(value: impl Into<String>) -> Result<Self, ProfileValidationError> {
        let value = value.into();
        if !is_stable_identifier(&value) || looks_like_credential(&value) {
            return Err(ProfileValidationError::InvalidProfileId);
        }
        Ok(Self(value))
    }

    /// 返回持久化使用的标识值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 引用由原生宿主安全保存的凭据。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SecretRef(String);

/// 标识允许创建秘密引用的平台存储命名空间。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SecretRefNamespace {
    /// Android Keystore 支持的宿主秘密存储。
    AndroidKeystore,
    /// macOS Keychain 支持的宿主秘密存储。
    MacosKeychain,
    /// Linux Secret Service 支持的宿主秘密存储。
    SecretService,
    /// 仅当前进程存活期间有效的内存秘密存储。
    Session,
    /// 确定性测试宿主使用的隔离秘密存储。
    TestSecret,
    /// Windows Credential Manager 支持的宿主秘密存储。
    WindowsCredential,
}

impl SecretRefNamespace {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AndroidKeystore => "android-keystore",
            Self::MacosKeychain => "macos-keychain",
            Self::SecretService => "secret-service",
            Self::Session => "session",
            Self::TestSecret => "test-secret",
            Self::WindowsCredential => "windows-credential",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "android-keystore" => Some(Self::AndroidKeystore),
            "macos-keychain" => Some(Self::MacosKeychain),
            "secret-service" => Some(Self::SecretService),
            "session" => Some(Self::Session),
            "test-secret" => Some(Self::TestSecret),
            "windows-credential" => Some(Self::WindowsCredential),
            _ => None,
        }
    }
}

impl SecretRef {
    /// 创建只包含宿主命名空间和随机不透明标识的新引用。
    #[must_use]
    pub fn new(namespace: SecretRefNamespace) -> Self {
        Self(format!("{}:{}", namespace.as_str(), Uuid::new_v4()))
    }

    /// 解析不包含凭据值的稳定秘密引用。
    pub fn parse(value: impl Into<String>) -> Result<Self, ProfileValidationError> {
        let value = value.into();
        if value.len() > MAX_STABLE_ID_BYTES {
            return Err(ProfileValidationError::InvalidSecretRef);
        }
        let Some((namespace, opaque_id)) = value.split_once(':') else {
            return Err(ProfileValidationError::InvalidSecretRef);
        };
        let parsed_id = Uuid::parse_str(opaque_id);
        if SecretRefNamespace::parse(namespace).is_none()
            || opaque_id.len() != 36
            || !matches!(parsed_id, Ok(id) if id.get_version() == Some(Version::Random) && id.get_variant() == Variant::RFC4122 && id.hyphenated().to_string() == opaque_id)
        {
            return Err(ProfileValidationError::InvalidSecretRef);
        }
        Ok(Self(value))
    }

    /// 返回供宿主秘密服务查找的引用值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 返回宿主实现选择使用的稳定命名空间。
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.0
            .split_once(':')
            .map_or("", |(namespace, _)| namespace)
    }

    /// 判断引用是否可以跨进程重启持久化。
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.namespace() != "session"
    }
}

/// 包装通过统一安全策略验证并规范化的提供商端点。
#[derive(Clone, Eq, PartialEq)]
pub struct EndpointConfiguration(String);

impl EndpointConfiguration {
    /// 仅接受远程 HTTPS 或回环 HTTP，拒绝嵌入秘密和签名查询。
    pub fn parse(value: impl Into<String>) -> Result<Self, ProfileValidationError> {
        let value = value.into();
        if value.len() > MAX_PROFILE_TEXT_BYTES || looks_like_credential(&value) {
            return Err(ProfileValidationError::InvalidEndpoint);
        }
        let mut url = Url::parse(&value).map_err(|_| ProfileValidationError::InvalidEndpoint)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.host().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path().contains('%')
        {
            return Err(ProfileValidationError::InvalidEndpoint);
        }
        match url.scheme() {
            "https" => {}
            "http" if is_loopback_endpoint(&url) => {}
            _ => return Err(ProfileValidationError::InvalidEndpoint),
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        if url
            .path_segments()
            .is_some_and(|segments| segments.into_iter().any(looks_like_credential))
        {
            return Err(ProfileValidationError::InvalidEndpoint);
        }
        Ok(Self(url.to_string()))
    }

    /// 返回提供商适配器使用的规范地址。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EndpointConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EndpointConfiguration([REDACTED])")
    }
}

/// 包装仅驻留内存且在释放时清零的宿主秘密值。
pub struct SecretValue(SecretString);

impl SecretValue {
    /// 接管一次性宿主响应中的秘密文本。
    #[must_use]
    pub fn new(value: impl Into<Box<str>>) -> Self {
        Self(SecretString::from(value.into()))
    }

    /// 仅向需要构造认证请求的适配器暴露秘密。
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

/// 表示一次性解析且仅用于代理认证的内存秘密。
pub struct ProxyAuthentication {
    username: String,
    password: SecretString,
}

/// 表示一次性解析且仅用于 TLS 客户端证书认证的内存身份。
pub struct ClientCertificateIdentity(SecretString);

impl ClientCertificateIdentity {
    /// 从受宿主保护的 PEM 文本解析客户端证书和私钥身份。
    pub fn parse(secret: &SecretValue) -> Result<Self, TranslationError> {
        let raw = secret.expose_secret();
        if raw.len() > MAX_PROVIDER_CLIENT_CERTIFICATE_IDENTITY_PEM_BYTES
            || raw.contains('\0')
            || raw
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            || !raw.contains("-----BEGIN CERTIFICATE-----")
            || !(raw.contains("-----BEGIN PRIVATE KEY-----")
                || raw.contains("-----BEGIN RSA PRIVATE KEY-----")
                || raw.contains("-----BEGIN EC PRIVATE KEY-----"))
        {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider client certificate identity is invalid.",
            ));
        }
        Ok(Self(SecretString::from(raw.to_owned())))
    }

    /// 返回仅用于构造 TLS 身份的 PEM 文本。
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for ClientCertificateIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClientCertificateIdentity([REDACTED])")
    }
}

impl ProxyAuthentication {
    /// 从受宿主保护的 `username:password` 文本解析代理认证。
    pub fn parse(secret: &SecretValue) -> Result<Self, TranslationError> {
        let raw = secret.expose_secret();
        if raw.len() > MAX_PROXY_AUTHENTICATION_BYTES
            || raw.chars().any(char::is_control)
            || raw.contains('\0')
        {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider proxy credentials are invalid.",
            ));
        }
        let Some((username, password)) = raw.split_once(':') else {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider proxy credentials are invalid.",
            ));
        };
        if username.is_empty()
            || password.is_empty()
            || username.len() > MAX_PROXY_AUTH_USERNAME_BYTES
            || password.len() > MAX_PROXY_AUTH_PASSWORD_BYTES
        {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider proxy credentials are invalid.",
            ));
        }
        Ok(Self {
            username: username.to_owned(),
            password: SecretString::from(password.to_owned()),
        })
    }

    /// 返回代理用户名。
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// 返回仅用于构造代理认证头的密码。
    #[must_use]
    pub fn password(&self) -> &str {
        self.password.expose_secret()
    }
}

impl fmt::Debug for ProxyAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProxyAuthentication([REDACTED])")
    }
}

/// 描述不含任何凭据值的规范提供商配置。
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderProfile {
    id: ProviderProfileId,
    display_name: String,
    preset_id: String,
    adapter_type: String,
    base_endpoint: EndpointConfiguration,
    secret_ref: Option<SecretRef>,
    secret_custom_headers_ref: Option<Box<SecretRef>>,
    user_notes: Option<String>,
    organization: Option<String>,
    project: Option<String>,
    region: Option<String>,
    account_identifier: Option<String>,
    custom_headers: Option<String>,
    proxy_url: Option<String>,
    proxy_auth_ref: Option<Box<SecretRef>>,
    request_timeout_secs: u32,
    connection_timeout_secs: u32,
    streaming_idle_timeout_secs: u32,
    trusted_certificates_pem: Option<String>,
    client_certificate_identity_ref: Option<Box<SecretRef>>,
    enabled: bool,
    selected_model: Option<String>,
}

impl fmt::Debug for ProviderProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderProfile")
            .field("id", &self.id)
            .field("preset_id", &self.preset_id)
            .field("adapter_type", &self.adapter_type)
            .field("base_endpoint", &"[REDACTED]")
            .field("has_secret_ref", &self.secret_ref.is_some())
            .field(
                "has_secret_custom_headers_ref",
                &self.secret_custom_headers_ref.is_some(),
            )
            .field("has_user_notes", &self.user_notes.is_some())
            .field("has_organization", &self.organization.is_some())
            .field("has_project", &self.project.is_some())
            .field("has_region", &self.region.is_some())
            .field("has_account_identifier", &self.account_identifier.is_some())
            .field("has_custom_headers", &self.custom_headers.is_some())
            .field("has_proxy_url", &self.proxy_url.is_some())
            .field("has_proxy_auth_ref", &self.proxy_auth_ref.is_some())
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("connection_timeout_secs", &self.connection_timeout_secs)
            .field(
                "streaming_idle_timeout_secs",
                &self.streaming_idle_timeout_secs,
            )
            .field(
                "has_trusted_certificates_pem",
                &self.trusted_certificates_pem.is_some(),
            )
            .field(
                "has_client_certificate_identity_ref",
                &self.client_certificate_identity_ref.is_some(),
            )
            .field("enabled", &self.enabled)
            .field("has_selected_model", &self.selected_model.is_some())
            .finish_non_exhaustive()
    }
}

impl ProviderProfile {
    /// 创建经过基础约束验证的非秘密提供商配置。
    pub fn new(
        id: ProviderProfileId,
        display_name: impl Into<String>,
        preset_id: impl Into<String>,
        adapter_type: impl Into<String>,
        base_endpoint: impl Into<String>,
        secret_ref: Option<SecretRef>,
    ) -> Result<Self, ProfileValidationError> {
        let display_name = checked_profile_text(display_name.into(), "display_name")?;
        let preset_id = checked_profile_text(preset_id.into(), "preset_id")?;
        let adapter_type = checked_profile_text(adapter_type.into(), "adapter_type")?;
        let base_endpoint = EndpointConfiguration::parse(base_endpoint)?;
        Ok(Self {
            id,
            display_name,
            preset_id,
            adapter_type,
            base_endpoint,
            secret_ref,
            secret_custom_headers_ref: None,
            user_notes: None,
            organization: None,
            project: None,
            region: None,
            account_identifier: None,
            custom_headers: None,
            proxy_url: None,
            proxy_auth_ref: None,
            request_timeout_secs: DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS,
            connection_timeout_secs: DEFAULT_PROVIDER_CONNECTION_TIMEOUT_SECS,
            streaming_idle_timeout_secs: DEFAULT_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS,
            trusted_certificates_pem: None,
            client_certificate_identity_ref: None,
            enabled: true,
            selected_model: None,
        })
    }

    /// 返回稳定配置标识。
    #[must_use]
    pub const fn id(&self) -> &ProviderProfileId {
        &self.id
    }

    /// 返回用户可见名称。
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 返回目录预设标识。
    #[must_use]
    pub fn preset_id(&self) -> &str {
        &self.preset_id
    }

    /// 返回核心适配器类型。
    #[must_use]
    pub fn adapter_type(&self) -> &str {
        &self.adapter_type
    }

    /// 返回不含嵌入凭据的基础端点。
    #[must_use]
    pub fn base_endpoint(&self) -> &str {
        self.base_endpoint.as_str()
    }

    /// 返回可选宿主秘密引用。
    #[must_use]
    pub const fn secret_ref(&self) -> Option<&SecretRef> {
        self.secret_ref.as_ref()
    }

    /// 返回可选宿主秘密自定义请求头引用。
    #[must_use]
    pub fn secret_custom_headers_ref(&self) -> Option<&SecretRef> {
        self.secret_custom_headers_ref.as_deref()
    }

    /// 设置只保存引用、不保存秘密值的自定义请求头引用。
    #[must_use]
    pub fn with_secret_custom_headers_ref(mut self, secret_ref: Option<SecretRef>) -> Self {
        self.secret_custom_headers_ref = secret_ref.map(Box::new);
        self
    }

    /// 返回用户填写的非秘密备注。
    #[must_use]
    pub fn user_notes(&self) -> Option<&str> {
        self.user_notes.as_deref()
    }

    /// 返回可选的非秘密组织标识。
    #[must_use]
    pub fn organization(&self) -> Option<&str> {
        self.organization.as_deref()
    }

    /// 返回可选的非秘密项目标识。
    #[must_use]
    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// 返回可选的非秘密区域标识。
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// 返回可选的非秘密账户标识。
    #[must_use]
    pub fn account_identifier(&self) -> Option<&str> {
        self.account_identifier.as_deref()
    }

    /// 返回规范化的非秘密自定义请求头 JSON。
    #[must_use]
    pub fn custom_headers(&self) -> Option<&str> {
        self.custom_headers.as_deref()
    }

    /// 返回不含凭据的可选代理地址。
    #[must_use]
    pub fn proxy_url(&self) -> Option<&str> {
        self.proxy_url.as_deref()
    }

    /// 返回可选的宿主代理认证秘密引用。
    #[must_use]
    pub fn proxy_auth_ref(&self) -> Option<&SecretRef> {
        self.proxy_auth_ref.as_deref()
    }

    /// 返回提供商请求的有界总超时秒数。
    #[must_use]
    pub const fn request_timeout_secs(&self) -> u32 {
        self.request_timeout_secs
    }

    /// 返回提供商连接建立的有界超时秒数。
    #[must_use]
    pub const fn connection_timeout_secs(&self) -> u32 {
        self.connection_timeout_secs
    }

    /// 返回提供商流式响应的有界空闲超时秒数。
    #[must_use]
    pub const fn streaming_idle_timeout_secs(&self) -> u32 {
        self.streaming_idle_timeout_secs
    }

    /// 返回可选的自定义可信证书 PEM 文本。
    #[must_use]
    pub fn trusted_certificates_pem(&self) -> Option<&str> {
        self.trusted_certificates_pem.as_deref()
    }

    /// 返回可选的宿主 TLS 客户端证书身份秘密引用。
    #[must_use]
    pub fn client_certificate_identity_ref(&self) -> Option<&SecretRef> {
        self.client_certificate_identity_ref.as_deref()
    }

    /// 返回配置是否允许被选择。
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// 返回该提供商最近明确选择的模型。
    #[must_use]
    pub fn selected_model(&self) -> Option<&str> {
        self.selected_model.as_deref()
    }

    /// 设置持久化启用状态。
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 设置该提供商最近明确选择的模型。
    pub fn with_selected_model(
        mut self,
        model_id: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.selected_model = model_id
            .map(|value| checked_profile_text(value, "selected_model"))
            .transpose()?;
        Ok(self)
    }

    /// 设置有界且不含凭据的用户备注。
    pub fn with_user_notes(
        mut self,
        notes: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.user_notes = notes
            .filter(|value| !value.trim().is_empty())
            .map(|value| checked_profile_text(value, "user_notes"))
            .transpose()?;
        Ok(self)
    }

    /// 设置有界且不含凭据的组织标识。
    pub fn with_organization(
        mut self,
        organization: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.organization = organization
            .filter(|value| !value.trim().is_empty())
            .map(|value| checked_profile_text(value, "organization"))
            .transpose()?;
        Ok(self)
    }

    /// 设置有界且不含凭据的项目标识。
    pub fn with_project(mut self, project: Option<String>) -> Result<Self, ProfileValidationError> {
        self.project = project
            .filter(|value| !value.trim().is_empty())
            .map(|value| checked_profile_text(value, "project"))
            .transpose()?;
        Ok(self)
    }

    /// 设置有界且不含凭据的区域标识。
    pub fn with_region(mut self, region: Option<String>) -> Result<Self, ProfileValidationError> {
        self.region = region
            .filter(|value| !value.trim().is_empty())
            .map(|value| checked_profile_text(value, "region"))
            .transpose()?;
        Ok(self)
    }

    /// 设置有界且不含凭据的账户标识。
    pub fn with_account_identifier(
        mut self,
        account_identifier: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.account_identifier = account_identifier
            .filter(|value| !value.trim().is_empty())
            .map(|value| checked_profile_text(value, "account_identifier"))
            .transpose()?;
        Ok(self)
    }

    /// 设置受限且不含凭据的自定义请求头 JSON。
    pub fn with_custom_headers(
        mut self,
        custom_headers: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.custom_headers = custom_headers
            .filter(|value| !value.trim().is_empty())
            .map(|value| validate_custom_headers(&value))
            .transpose()?;
        Ok(self)
    }

    /// 设置受限且不含凭据的代理地址。
    pub fn with_proxy_url(
        mut self,
        proxy_url: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.proxy_url = proxy_url
            .filter(|value| !value.trim().is_empty())
            .map(|value| validate_proxy_url(&value))
            .transpose()?;
        Ok(self)
    }

    /// 设置只保存引用、不保存代理认证值的宿主秘密引用。
    #[must_use]
    pub fn with_proxy_auth_ref(mut self, secret_ref: Option<SecretRef>) -> Self {
        self.proxy_auth_ref = secret_ref.map(Box::new);
        self
    }

    /// 设置提供商请求的有界总超时秒数。
    pub fn with_request_timeout_secs(
        mut self,
        request_timeout_secs: u32,
    ) -> Result<Self, ProfileValidationError> {
        if !(MIN_PROVIDER_REQUEST_TIMEOUT_SECS..=MAX_PROVIDER_REQUEST_TIMEOUT_SECS)
            .contains(&request_timeout_secs)
        {
            return Err(ProfileValidationError::InvalidField("request_timeout_secs"));
        }
        self.request_timeout_secs = request_timeout_secs;
        Ok(self)
    }

    /// 设置提供商连接建立的有界超时秒数。
    pub fn with_connection_timeout_secs(
        mut self,
        connection_timeout_secs: u32,
    ) -> Result<Self, ProfileValidationError> {
        if !(MIN_PROVIDER_CONNECTION_TIMEOUT_SECS..=MAX_PROVIDER_CONNECTION_TIMEOUT_SECS)
            .contains(&connection_timeout_secs)
        {
            return Err(ProfileValidationError::InvalidField(
                "connection_timeout_secs",
            ));
        }
        self.connection_timeout_secs = connection_timeout_secs;
        Ok(self)
    }

    /// 设置提供商流式响应的有界空闲超时秒数。
    pub fn with_streaming_idle_timeout_secs(
        mut self,
        streaming_idle_timeout_secs: u32,
    ) -> Result<Self, ProfileValidationError> {
        if !(MIN_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS..=MAX_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS)
            .contains(&streaming_idle_timeout_secs)
        {
            return Err(ProfileValidationError::InvalidField(
                "streaming_idle_timeout_secs",
            ));
        }
        self.streaming_idle_timeout_secs = streaming_idle_timeout_secs;
        Ok(self)
    }

    /// 设置显式配置的自定义可信证书 PEM 文本；系统证书仍然保留。
    pub fn with_trusted_certificates_pem(
        mut self,
        trusted_certificates_pem: Option<String>,
    ) -> Result<Self, ProfileValidationError> {
        self.trusted_certificates_pem = trusted_certificates_pem
            .filter(|value| !value.trim().is_empty())
            .map(|value| validate_trusted_certificates_pem(&value))
            .transpose()?;
        Ok(self)
    }

    /// 设置只保存引用、不保存 TLS 客户端证书和私钥的宿主秘密引用。
    #[must_use]
    pub fn with_client_certificate_identity_ref(mut self, secret_ref: Option<SecretRef>) -> Self {
        self.client_certificate_identity_ref = secret_ref.map(Box::new);
        self
    }
}

/// 新建提供商配置时使用的安全请求超时。
pub const DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS: u32 = 30;
/// 提供商请求超时的最小秒数。
pub const MIN_PROVIDER_REQUEST_TIMEOUT_SECS: u32 = 1;
/// 提供商请求超时的最大秒数。
pub const MAX_PROVIDER_REQUEST_TIMEOUT_SECS: u32 = 600;
/// 新建提供商配置时使用的安全连接超时。
pub const DEFAULT_PROVIDER_CONNECTION_TIMEOUT_SECS: u32 = 10;
/// 提供商连接超时的最小秒数。
pub const MIN_PROVIDER_CONNECTION_TIMEOUT_SECS: u32 = 1;
/// 提供商连接超时的最大秒数。
pub const MAX_PROVIDER_CONNECTION_TIMEOUT_SECS: u32 = 120;
/// 新建提供商配置时使用的安全流式空闲超时。
pub const DEFAULT_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS: u32 = 60;
/// 提供商流式空闲超时的最小秒数。
pub const MIN_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS: u32 = 1;
/// 提供商流式空闲超时的最大秒数。
pub const MAX_PROVIDER_STREAMING_IDLE_TIMEOUT_SECS: u32 = 300;
/// 自定义可信证书 PEM 文本的最大字节数。
pub const MAX_PROVIDER_TRUSTED_CERTIFICATES_PEM_BYTES: usize = 64 * 1024;
/// TLS 客户端证书和私钥 PEM 身份的最大字节数。
pub const MAX_PROVIDER_CLIENT_CERTIFICATE_IDENTITY_PEM_BYTES: usize = 128 * 1024;
/// 代理认证用户名的最大字节数。
pub const MAX_PROXY_AUTH_USERNAME_BYTES: usize = 256;
/// 代理认证密码的最大字节数。
pub const MAX_PROXY_AUTH_PASSWORD_BYTES: usize = 4096;
/// 代理认证秘密文本的最大字节数。
pub const MAX_PROXY_AUTHENTICATION_BYTES: usize =
    MAX_PROXY_AUTH_USERNAME_BYTES + 1 + MAX_PROXY_AUTH_PASSWORD_BYTES;

/// 验证即将写入配置存储的模型标识不包含凭据形态。
pub fn validate_model_identifier(value: &str) -> Result<(), ProfileValidationError> {
    checked_profile_text(value.to_owned(), "model_id").map(drop)
}

/// 验证跨原生协议传输的非秘密提供商字段，并返回其原始值。
pub fn validate_non_secret_profile_field(
    value: &str,
    field: &'static str,
) -> Result<String, ProfileValidationError> {
    checked_profile_text(value.to_owned(), field)
}

/// 描述规范提供商配置验证失败。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProfileValidationError {
    /// 配置标识不符合稳定格式。
    #[error("Provider profile ID is invalid.")]
    InvalidProfileId,
    /// 秘密引用不符合稳定格式。
    #[error("Provider secret reference is invalid.")]
    InvalidSecretRef,
    /// 端点违反统一传输和秘密保护策略。
    #[error("Provider endpoint is invalid or unsafe.")]
    InvalidEndpoint,
    /// 必填文本为空或超过限制。
    #[error("Provider profile field is invalid: {0}.")]
    InvalidField(&'static str),
    /// 非秘密字段疑似包含凭据值。
    #[error("Provider profile field resembles a credential: {0}.")]
    CredentialLikeValue(&'static str),
}

/// 描述客户端启动时查询到的共享核心契约。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreCompatibility {
    /// 共享核心语义版本。
    pub core_version: String,
    /// 稳定原生 ABI 主版本。
    pub abi_major: u32,
    /// 命令和事件协议版本。
    pub protocol_version: u32,
    /// 内置提供商目录语义版本。
    pub provider_catalog_version: String,
    /// 已启用且可由客户端探测的稳定功能标识。
    pub enabled_features: Vec<String>,
}

/// 描述客户端可接受的预发布核心版本和必需功能。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityRequirements {
    /// 客户端审查过的核心语义版本。
    pub core_version: String,
    /// 客户端支持的 ABI 主版本。
    pub abi_major: u32,
    /// 客户端支持的协议版本。
    pub protocol_version: u32,
    /// 客户端审查过的提供商目录版本。
    pub provider_catalog_version: String,
    /// 客户端必须使用的功能集合。
    pub required_features: Vec<String>,
}

impl CompatibilityRequirements {
    /// 精确验证版本和目录维度，并要求已启用功能包含必需子集。
    pub fn validate(&self, actual: &CoreCompatibility) -> Result<(), CompatibilityError> {
        if actual.core_version != self.core_version {
            return Err(CompatibilityError::CoreVersion {
                expected: self.core_version.clone(),
                actual: actual.core_version.clone(),
            });
        }
        if actual.abi_major != self.abi_major {
            return Err(CompatibilityError::AbiMajor {
                expected: self.abi_major,
                actual: actual.abi_major,
            });
        }
        if actual.protocol_version != self.protocol_version {
            return Err(CompatibilityError::ProtocolVersion {
                expected: self.protocol_version,
                actual: actual.protocol_version,
            });
        }
        if actual.provider_catalog_version != self.provider_catalog_version {
            return Err(CompatibilityError::ProviderCatalogVersion {
                expected: self.provider_catalog_version.clone(),
                actual: actual.provider_catalog_version.clone(),
            });
        }
        for required in &self.required_features {
            if !actual.enabled_features.contains(required) {
                return Err(CompatibilityError::MissingFeature(required.clone()));
            }
        }
        Ok(())
    }
}

/// 描述客户端拒绝不兼容共享核心的安全原因。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CompatibilityError {
    /// 核心语义版本未经客户端审查。
    #[error("Core version is incompatible: expected {expected}, received {actual}.")]
    CoreVersion {
        /// 客户端要求的版本。
        expected: String,
        /// 核心报告的版本。
        actual: String,
    },
    /// ABI 主版本不受支持。
    #[error("Core ABI is incompatible: expected {expected}, received {actual}.")]
    AbiMajor {
        /// 客户端支持的版本。
        expected: u32,
        /// 核心报告的版本。
        actual: u32,
    },
    /// 命令和事件协议不受支持。
    #[error("Core protocol is incompatible: expected {expected}, received {actual}.")]
    ProtocolVersion {
        /// 客户端支持的版本。
        expected: u32,
        /// 核心报告的版本。
        actual: u32,
    },
    /// 提供商目录未经客户端审查。
    #[error("Provider catalog is incompatible: expected {expected}, received {actual}.")]
    ProviderCatalogVersion {
        /// 客户端要求的版本。
        expected: String,
        /// 核心报告的版本。
        actual: String,
    },
    /// 核心缺少客户端必需功能。
    #[error("Core feature is unavailable: {0}.")]
    MissingFeature(String),
}

fn is_stable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_STABLE_ID_BYTES
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn checked_profile_text(
    value: String,
    field: &'static str,
) -> Result<String, ProfileValidationError> {
    if value.trim().is_empty() || value.len() > MAX_PROFILE_TEXT_BYTES || value.contains('\0') {
        Err(ProfileValidationError::InvalidField(field))
    } else if looks_like_credential(&value) {
        Err(ProfileValidationError::CredentialLikeValue(field))
    } else {
        Ok(value)
    }
}

fn validate_custom_headers(value: &str) -> Result<String, ProfileValidationError> {
    let headers = serde_json::from_str::<std::collections::BTreeMap<String, String>>(value)
        .map_err(|_| ProfileValidationError::InvalidField("custom_headers"))?;
    if headers.is_empty() || headers.len() > MAX_CUSTOM_HEADERS {
        return Err(ProfileValidationError::InvalidField("custom_headers"));
    }
    for (name, header_value) in &headers {
        if name.is_empty()
            || name.len() > MAX_CUSTOM_HEADER_NAME_BYTES
            || !name.bytes().all(is_http_token_byte)
            || is_secret_header_name(name)
            || header_value.is_empty()
            || header_value.len() > MAX_CUSTOM_HEADER_VALUE_BYTES
            || header_value.chars().any(char::is_control)
            || looks_like_credential(header_value)
        {
            return Err(ProfileValidationError::InvalidField("custom_headers"));
        }
    }
    serde_json::to_string(&headers)
        .map_err(|_| ProfileValidationError::InvalidField("custom_headers"))
}

fn validate_proxy_url(value: &str) -> Result<String, ProfileValidationError> {
    if value.len() > MAX_PROXY_URL_BYTES || value.chars().any(char::is_control) {
        return Err(ProfileValidationError::InvalidField("proxy_url"));
    }
    let url = Url::parse(value).map_err(|_| ProfileValidationError::InvalidField("proxy_url"))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!url.path().is_empty() && url.path() != "/")
    {
        return Err(ProfileValidationError::InvalidField("proxy_url"));
    }
    Ok(url.to_string())
}

fn validate_trusted_certificates_pem(value: &str) -> Result<String, ProfileValidationError> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.len() > MAX_PROVIDER_TRUSTED_CERTIFICATES_PEM_BYTES
        || normalized.contains('\0')
        || normalized.contains("PRIVATE KEY")
        || !normalized.contains("-----BEGIN CERTIFICATE-----")
        || !normalized.contains("-----END CERTIFICATE-----")
        || normalized
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(ProfileValidationError::InvalidField(
            "trusted_certificates_pem",
        ));
    }
    Ok(normalized.trim().to_owned())
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

fn is_secret_header_name(name: &str) -> bool {
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

fn looks_like_credential(value: &str) -> bool {
    let value = value.trim();
    contains_credential_token(value, "sk-", 20)
        || contains_credential_token(value, "ghp_", 24)
        || value.contains(concat!("github_", "pat_"))
        || contains_bearer_token(value)
        || value.contains("PRIVATE KEY-----")
}

fn contains_credential_token(value: &str, prefix: &str, minimum_length: usize) -> bool {
    value.match_indices(prefix).any(|(start, _)| {
        value[start..]
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            .count()
            >= minimum_length
    })
}

fn contains_bearer_token(value: &str) -> bool {
    const PREFIX: &str = "Bearer ";
    value.match_indices(PREFIX).any(|(start, _)| {
        value[start + PREFIX.len()..]
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            .count()
            >= 20
    })
}

fn is_loopback_endpoint(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

const PROTECTED_TOKEN_PREFIX: &str = "__LINGUAMESH_PROTECTED_";
const PROTECTED_TOKEN_SUFFIX: &str = "__";
/// 未提供 tokenizer 时使用的保守近似分段上限。
pub const DEFAULT_TRANSLATION_CHUNK_BYTES: usize = 16 * 1024;
const MAX_GLOSSARY_ENTRIES: usize = 256;
const MAX_GLOSSARY_TERM_BYTES: usize = 512;
const MAX_GLOSSARY_NOTE_BYTES: usize = 2048;
/// 词汇表 CSV 文件的最大 UTF-8 字节数。
pub const MAX_GLOSSARY_CSV_BYTES: usize = 4 * 1024 * 1024;
/// 词汇表 CSV 的固定列顺序。
pub const GLOSSARY_CSV_HEADER: &str = "source_term,target_term,source_locale,target_locale,case_sensitive,whole_word,immutable,domain,priority,notes,enabled";

/// 表示一条请求级词汇表规则。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct GlossaryEntry {
    /// 源语言术语。
    pub source_term: String,
    /// 必须使用的目标语言术语。
    pub target_term: String,
    /// 可选的源语言范围。
    pub source_locale: Option<String>,
    /// 可选的目标语言范围。
    pub target_locale: Option<String>,
    /// 是否区分源术语大小写。
    pub case_sensitive: bool,
    /// 是否要求源术语位于词边界。
    pub whole_word: bool,
    /// 是否原样保留源术语而不替换为目标术语。
    pub immutable: bool,
    /// 可选的领域标签。
    pub domain: Option<String>,
    /// 冲突时使用的确定性优先级。
    pub priority: i32,
    /// 供用户查看的非秘密备注。
    pub notes: Option<String>,
    /// 是否将此规则用于当前请求。
    pub enabled: bool,
}

impl GlossaryEntry {
    /// 创建启用、全词匹配的必需目标译法规则。
    pub fn new(
        source_term: impl Into<String>,
        target_term: impl Into<String>,
    ) -> Result<Self, GlossaryError> {
        let entry = Self {
            source_term: source_term.into(),
            target_term: target_term.into(),
            source_locale: None,
            target_locale: None,
            case_sensitive: true,
            whole_word: true,
            immutable: false,
            domain: None,
            priority: 0,
            notes: None,
            enabled: true,
        };
        entry.validate()?;
        Ok(entry)
    }

    /// 设置源语言范围。
    #[must_use]
    pub fn with_source_locale(mut self, locale: impl Into<String>) -> Self {
        self.source_locale = Some(locale.into());
        self
    }

    /// 设置目标语言范围。
    #[must_use]
    pub fn with_target_locale(mut self, locale: impl Into<String>) -> Self {
        self.target_locale = Some(locale.into());
        self
    }

    /// 设置是否区分大小写。
    #[must_use]
    pub const fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// 设置是否要求词边界。
    #[must_use]
    pub const fn with_whole_word(mut self, whole_word: bool) -> Self {
        self.whole_word = whole_word;
        self
    }

    /// 设置是否原样保护源术语。
    #[must_use]
    pub const fn with_immutable(mut self, immutable: bool) -> Self {
        self.immutable = immutable;
        self
    }

    /// 设置领域标签。
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// 设置确定性优先级。
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 设置非秘密备注。
    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// 设置规则是否启用。
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn validate(&self) -> Result<(), GlossaryError> {
        validate_glossary_text(&self.source_term, MAX_GLOSSARY_TERM_BYTES, "source term")?;
        validate_glossary_text(&self.target_term, MAX_GLOSSARY_TERM_BYTES, "target term")?;
        if let Some(locale) = &self.source_locale {
            validate_glossary_text(locale, 64, "source locale")?;
        }
        if let Some(locale) = &self.target_locale {
            validate_glossary_text(locale, 64, "target locale")?;
        }
        if let Some(domain) = &self.domain {
            validate_glossary_text(domain, 128, "domain")?;
        }
        if let Some(notes) = &self.notes {
            validate_glossary_text(notes, MAX_GLOSSARY_NOTE_BYTES, "notes")?;
        }
        Ok(())
    }
}

/// 表示一组经过冲突校验的请求级词汇表规则。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Glossary {
    entries: Vec<GlossaryEntry>,
}

impl Glossary {
    /// 从规则列表创建词汇表并执行确定性冲突校验。
    pub fn new(entries: Vec<GlossaryEntry>) -> Result<Self, GlossaryError> {
        let glossary = Self { entries };
        glossary.validate()?;
        Ok(glossary)
    }

    /// 返回词汇表中的规则。
    #[must_use]
    pub fn entries(&self) -> &[GlossaryEntry] {
        &self.entries
    }

    /// 验证规则数量、字段安全性和同级冲突。
    pub fn validate(&self) -> Result<(), GlossaryError> {
        if self.entries.len() > MAX_GLOSSARY_ENTRIES {
            return Err(GlossaryError::TooManyEntries);
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        for (index, left) in self.entries.iter().enumerate() {
            if !left.enabled {
                continue;
            }
            for right in self.entries.iter().skip(index + 1) {
                if !right.enabled
                    || left.source_term != right.source_term
                    || left.case_sensitive != right.case_sensitive
                    || left.source_locale != right.source_locale
                    || left.target_locale != right.target_locale
                    || left.domain != right.domain
                    || left.target_term == right.target_term
                {
                    continue;
                }
                return Err(GlossaryError::ConflictingEntries);
            }
        }
        Ok(())
    }

    /// 从带有固定列头的 UTF-8 CSV 文件导入词汇表。
    pub fn from_csv(source: &str) -> Result<Self, GlossaryCsvError> {
        if source.len() > MAX_GLOSSARY_CSV_BYTES {
            return Err(GlossaryCsvError::TooLarge);
        }
        let mut records = parse_csv_records(source)?;
        if records.is_empty() {
            return Err(GlossaryCsvError::MissingHeader);
        }
        let header = records.remove(0);
        let expected = GLOSSARY_CSV_HEADER
            .split(',')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if header != expected {
            return Err(GlossaryCsvError::InvalidHeader);
        }
        if records.len() > MAX_GLOSSARY_ENTRIES {
            return Err(GlossaryCsvError::TooManyRows);
        }
        let mut entries = Vec::with_capacity(records.len());
        for (index, record) in records.into_iter().enumerate() {
            if record.len() != expected.len() {
                return Err(GlossaryCsvError::InvalidRow(index + 2));
            }
            entries.push(parse_glossary_entry(&record, index + 2)?);
        }
        Self::new(entries).map_err(GlossaryCsvError::Glossary)
    }

    /// 以固定列顺序导出可重复生成的 UTF-8 CSV 文件。
    #[must_use]
    pub fn to_csv(&self) -> String {
        let mut output = String::from(GLOSSARY_CSV_HEADER);
        output.push('\n');
        for entry in &self.entries {
            let row = [
                entry.source_term.as_str(),
                entry.target_term.as_str(),
                entry.source_locale.as_deref().unwrap_or_default(),
                entry.target_locale.as_deref().unwrap_or_default(),
                bool_to_csv(entry.case_sensitive),
                bool_to_csv(entry.whole_word),
                bool_to_csv(entry.immutable),
                entry.domain.as_deref().unwrap_or_default(),
                &entry.priority.to_string(),
                entry.notes.as_deref().unwrap_or_default(),
                bool_to_csv(entry.enabled),
            ];
            for (column, value) in row.iter().enumerate() {
                if column > 0 {
                    output.push(',');
                }
                append_csv_field(&mut output, value);
            }
            output.push('\n');
        }
        output
    }
}

/// 描述词汇表 CSV 输入不满足安全或结构约束的原因。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GlossaryCsvError {
    /// 文件超过允许的 UTF-8 字节上限。
    #[error("Glossary CSV exceeds the 4 MiB size limit.")]
    TooLarge,
    /// 文件没有列头。
    #[error("Glossary CSV is missing its header row.")]
    MissingHeader,
    /// 列头不是当前规范列头。
    #[error("Glossary CSV header does not match the supported schema.")]
    InvalidHeader,
    /// 数据行数量超过上限。
    #[error("Glossary CSV contains too many rows.")]
    TooManyRows,
    /// CSV 引号或换行结构无效。
    #[error("Glossary CSV contains malformed quoting.")]
    Malformed,
    /// 数据行列数不正确。
    #[error("Glossary CSV row {0} has an invalid column count.")]
    InvalidRow(usize),
    /// 布尔值、优先级或字段内容无效。
    #[error("Glossary CSV row {0} contains an invalid value.")]
    InvalidValue(usize),
    /// 行中的词汇表规则未通过领域校验。
    #[error("Glossary CSV row {row} is invalid: {error}")]
    InvalidEntry { row: usize, error: GlossaryError },
    /// 导入后的词汇表存在冲突。
    #[error("Glossary CSV contains invalid glossary rules: {0}")]
    Glossary(GlossaryError),
}

fn parse_glossary_entry(record: &[String], row: usize) -> Result<GlossaryEntry, GlossaryCsvError> {
    let parse_bool = |value: &str| match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(GlossaryCsvError::InvalidValue(row)),
    };
    let mut entry = GlossaryEntry::new(&record[0], &record[1])
        .map_err(|error| GlossaryCsvError::InvalidEntry { row, error })?;
    if !record[2].is_empty() {
        entry = entry.with_source_locale(&record[2]);
    }
    if !record[3].is_empty() {
        entry = entry.with_target_locale(&record[3]);
    }
    entry = entry
        .with_case_sensitive(parse_bool(&record[4])?)
        .with_whole_word(parse_bool(&record[5])?)
        .with_immutable(parse_bool(&record[6])?);
    if !record[7].is_empty() {
        entry = entry.with_domain(&record[7]);
    }
    entry = entry.with_priority(
        record[8]
            .parse::<i32>()
            .map_err(|_| GlossaryCsvError::InvalidValue(row))?,
    );
    if !record[9].is_empty() {
        entry = entry.with_notes(&record[9]);
    }
    entry = entry.with_enabled(parse_bool(&record[10])?);
    entry
        .validate()
        .map_err(|error| GlossaryCsvError::InvalidEntry { row, error })?;
    Ok(entry)
}

fn bool_to_csv(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn append_csv_field(output: &mut String, value: &str) {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        output.push('"');
        for character in value.chars() {
            if character == '"' {
                output.push('"');
            }
            output.push(character);
        }
        output.push('"');
    } else {
        output.push_str(value);
    }
}

fn parse_csv_records(source: &str) -> Result<Vec<Vec<String>>, GlossaryCsvError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed_quote = false;
    let mut at_field_start = true;
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if quoted {
            match character {
                '"' if characters.peek() == Some(&'"') => {
                    field.push('"');
                    characters.next();
                }
                '"' => {
                    quoted = false;
                    closed_quote = true;
                }
                other => field.push(other),
            }
            continue;
        }
        if closed_quote {
            match character {
                ',' => {
                    record.push(std::mem::take(&mut field));
                    at_field_start = true;
                    closed_quote = false;
                }
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                    at_field_start = true;
                    closed_quote = false;
                }
                '\r' if characters.peek() == Some(&'\n') => {
                    characters.next();
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                    at_field_start = true;
                    closed_quote = false;
                }
                _ => return Err(GlossaryCsvError::Malformed),
            }
            continue;
        }
        match character {
            '"' if at_field_start => quoted = true,
            ',' => {
                record.push(std::mem::take(&mut field));
                at_field_start = true;
            }
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                at_field_start = true;
            }
            '\r' if characters.peek() == Some(&'\n') => {
                characters.next();
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                at_field_start = true;
            }
            '"' | '\r' => return Err(GlossaryCsvError::Malformed),
            other => {
                field.push(other);
                at_field_start = false;
            }
        }
    }
    if quoted {
        return Err(GlossaryCsvError::Malformed);
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

/// 描述词汇表输入不满足安全或确定性约束。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GlossaryError {
    /// 词汇表规则数量超过上限。
    #[error("Glossary contains too many entries.")]
    TooManyEntries,
    /// 词汇表字段无效。
    #[error("Glossary {0} is invalid.")]
    InvalidField(&'static str),
    /// 同一匹配范围存在不同目标译法。
    #[error("Glossary contains conflicting entries.")]
    ConflictingEntries,
    /// 词汇表字段疑似包含凭据。
    #[error("Glossary {0} contains credential-like data.")]
    CredentialLikeValue(&'static str),
}

fn validate_glossary_text(
    value: &str,
    max_bytes: usize,
    field: &'static str,
) -> Result<(), GlossaryError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.contains('\0') {
        return Err(GlossaryError::InvalidField(field));
    }
    if looks_like_credential(value) {
        return Err(GlossaryError::CredentialLikeValue(field));
    }
    Ok(())
}

/// 表示一个必须在翻译输出中原样恢复的非语言片段。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedSpan {
    /// 发送给模型的不可变占位符。
    token: String,
    /// 用户源文本中的原始片段。
    source: String,
    /// 恢复时输出的片段，词汇表规则可将源术语替换为必需目标术语。
    replacement: String,
}

/// 表示已替换受保护片段的源文本。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedSource {
    /// 发送给模型的安全文本。
    text: String,
    /// 待恢复的受保护片段。
    spans: Vec<ProtectedSpan>,
}

impl ProtectedSource {
    /// 返回发送给模型的安全文本。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 返回受保护片段数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// 判断源文本是否包含受保护片段。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// 创建与此源文本对应的增量恢复器。
    #[must_use]
    pub fn restorer(&self) -> ProtectedTextRestorer {
        ProtectedTextRestorer {
            spans: self.spans.clone(),
            pending: String::new(),
            seen: vec![false; self.spans.len()],
        }
    }

    /// 按安全字节上限和语义边界拆分受保护源文本。
    pub fn chunks(&self, max_bytes: usize) -> Result<Vec<Self>, ChunkingError> {
        let tokens = self
            .spans
            .iter()
            .map(|span| span.token.as_str())
            .collect::<Vec<_>>();
        let pieces = split_protected_text(&self.text, max_bytes, &tokens)?;
        let mut used = vec![false; self.spans.len()];
        let mut chunks = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let mut spans = Vec::new();
            for (index, span) in self.spans.iter().enumerate() {
                if piece.contains(&span.token) {
                    if used[index] {
                        return Err(ChunkingError::ProtectedSpanSplit);
                    }
                    used[index] = true;
                    spans.push(span.clone());
                }
            }
            chunks.push(Self { text: piece, spans });
        }
        if used.iter().any(|was_used| !was_used) {
            return Err(ChunkingError::ProtectedSpanSplit);
        }
        Ok(chunks)
    }
}

/// 描述长文本分段无法满足安全边界的原因。
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ChunkingError {
    /// 分段上限无效。
    #[error("Translation chunk size must be greater than zero.")]
    InvalidLimit,
    /// 单个受保护片段超过分段上限。
    #[error("A protected span exceeds the translation chunk size.")]
    ProtectedSpanTooLarge,
    /// 分段无法保持受保护片段完整。
    #[error("Translation chunking would split a protected span.")]
    ProtectedSpanSplit,
}

fn split_protected_text(
    text: &str,
    max_bytes: usize,
    protected_tokens: &[&str],
) -> Result<Vec<String>, ChunkingError> {
    if max_bytes == 0 {
        return Err(ChunkingError::InvalidLimit);
    }
    if text.is_empty() {
        return Ok(vec![String::new()]);
    }
    if text.len() <= max_bytes {
        return Ok(vec![text.to_owned()]);
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut limit = (start + max_bytes).min(text.len());
        while limit > start && !text.is_char_boundary(limit) {
            limit -= 1;
        }
        let crossing_span = protected_tokens.iter().find_map(|token| {
            let relative = text[start..].find(token)?;
            let token_start = start + relative;
            let token_end = token_start + token.len();
            (token_start < limit && token_end > limit).then_some((token_start, token_end))
        });
        if let Some((token_start, token_end)) = crossing_span {
            if token_start == start {
                return Err(ChunkingError::ProtectedSpanTooLarge);
            }
            limit = token_start;
            if token_end - token_start > max_bytes {
                return Err(ChunkingError::ProtectedSpanTooLarge);
            }
        }
        let boundary = semantic_boundary(text, start, limit);
        if boundary <= start {
            return Err(ChunkingError::ProtectedSpanSplit);
        }
        chunks.push(text[start..boundary].to_owned());
        start = boundary;
    }
    Ok(chunks)
}

fn semantic_boundary(text: &str, start: usize, limit: usize) -> usize {
    let mut preferred = None;
    for (relative, character) in text[start..limit].char_indices() {
        let end = start + relative + character.len_utf8();
        if matches!(character, '\n' | '.' | '!' | '?' | '。' | '！' | '？')
            || character.is_whitespace()
        {
            preferred = Some(end);
        }
    }
    preferred.unwrap_or(limit)
}

/// 扫描常见结构化片段并替换为不透明占位符。
#[must_use]
pub fn protect_source_text(source: &str) -> ProtectedSource {
    build_protected_source(source, None, None, None)
        .expect("automatic protected-span scanning cannot fail")
}

/// 按请求语言和词汇表规则保护源文本并返回可校验的恢复器。
pub fn protect_source_text_with_glossary(
    source: &str,
    source_locale: Option<&str>,
    target_locale: &str,
    glossary: Option<&Glossary>,
) -> Result<ProtectedSource, GlossaryError> {
    build_protected_source(source, source_locale, Some(target_locale), glossary)
}

struct ProtectedCandidate {
    start: usize,
    end: usize,
    replacement: String,
    priority: i32,
}

fn build_protected_source(
    source: &str,
    source_locale: Option<&str>,
    target_locale: Option<&str>,
    glossary: Option<&Glossary>,
) -> Result<ProtectedSource, GlossaryError> {
    if let Some(glossary) = glossary {
        glossary.validate()?;
    }
    let mut candidates = Vec::new();
    let mut automatic_candidates = Vec::new();
    collect_fenced_code_candidates(source, &mut automatic_candidates);
    collect_inline_code_candidates(source, &mut automatic_candidates);
    collect_placeholder_candidates(source, &mut automatic_candidates);
    collect_url_candidates(source, &mut automatic_candidates);
    collect_email_candidates(source, &mut automatic_candidates);
    candidates.extend(
        automatic_candidates
            .into_iter()
            .map(|(start, end)| ProtectedCandidate {
                start,
                end,
                replacement: source[start..end].to_owned(),
                priority: i32::MIN,
            }),
    );
    if let (Some(target_locale), Some(glossary)) = (target_locale, glossary) {
        collect_glossary_candidates(
            source,
            source_locale,
            target_locale,
            glossary,
            &mut candidates,
        );
    }

    candidates.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| (right.end - right.start).cmp(&(left.end - left.start)))
    });
    let mut selected = Vec::new();
    for candidate in candidates {
        if candidate.start < candidate.end
            && selected
                .last()
                .is_none_or(|previous: &ProtectedCandidate| candidate.start >= previous.end)
        {
            selected.push(candidate);
        }
    }

    let mut protected_text = String::with_capacity(source.len());
    let mut spans = Vec::with_capacity(selected.len());
    let mut cursor = 0;
    for (index, candidate) in selected.into_iter().enumerate() {
        protected_text.push_str(&source[cursor..candidate.start]);
        let mut token = format!("{PROTECTED_TOKEN_PREFIX}{index}{PROTECTED_TOKEN_SUFFIX}");
        while source.contains(&token) {
            token.push('_');
        }
        protected_text.push_str(&token);
        spans.push(ProtectedSpan {
            token,
            source: source[candidate.start..candidate.end].to_owned(),
            replacement: candidate.replacement,
        });
        cursor = candidate.end;
    }
    protected_text.push_str(&source[cursor..]);
    Ok(ProtectedSource {
        text: protected_text,
        spans,
    })
}

/// 描述模型输出中的受保护片段恢复失败。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProtectedTextError {
    /// 模型重复输出同一个占位符。
    #[error("Provider output repeated a protected marker.")]
    DuplicateMarker,
    /// 模型遗漏了一个占位符。
    #[error("Provider output omitted a protected marker.")]
    MissingMarker,
    /// 模型输出了未知占位符。
    #[error("Provider output contained an invalid protected marker.")]
    InvalidMarker,
}

/// 将流式模型输出中的占位符增量恢复为原始片段。
pub struct ProtectedTextRestorer {
    spans: Vec<ProtectedSpan>,
    pending: String,
    seen: Vec<bool>,
}

impl ProtectedTextRestorer {
    /// 消费一段模型增量并返回可安全展示的文本。
    pub fn push(&mut self, delta: &str) -> Result<String, ProtectedTextError> {
        self.pending.push_str(delta);
        if self.spans.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }

        let mut output = String::new();
        loop {
            let next_marker = self
                .spans
                .iter()
                .enumerate()
                .filter_map(|(index, span)| {
                    self.pending
                        .find(&span.token)
                        .map(|position| (position, index))
                })
                .min_by_key(|(position, _)| *position);
            let Some((position, index)) = next_marker else {
                let keep = longest_marker_prefix_suffix(&self.pending, &self.spans);
                let split = self.pending.len().saturating_sub(keep);
                let plain = self.pending[..split].to_owned();
                self.pending = self.pending[split..].to_owned();
                append_plain_text(&mut output, &plain)?;
                break;
            };
            if self.seen[index] {
                return Err(ProtectedTextError::DuplicateMarker);
            }
            let plain = self.pending[..position].to_owned();
            append_plain_text(&mut output, &plain)?;
            output.push_str(&self.spans[index].replacement);
            let end = position + self.spans[index].token.len();
            self.pending = self.pending[end..].to_owned();
            self.seen[index] = true;
        }
        Ok(output)
    }

    /// 完成输出并验证每个受保护片段均被恢复一次。
    pub fn finish(&mut self) -> Result<String, ProtectedTextError> {
        let mut output = self.push("")?;
        if !self.pending.is_empty() {
            let plain = std::mem::take(&mut self.pending);
            append_plain_text(&mut output, &plain)?;
        }
        if self.seen.iter().any(|seen| !seen) {
            return Err(ProtectedTextError::MissingMarker);
        }
        Ok(output)
    }
}

fn append_plain_text(output: &mut String, plain: &str) -> Result<(), ProtectedTextError> {
    if plain.contains(PROTECTED_TOKEN_PREFIX) {
        return Err(ProtectedTextError::InvalidMarker);
    }
    output.push_str(plain);
    Ok(())
}

fn longest_marker_prefix_suffix(text: &str, spans: &[ProtectedSpan]) -> usize {
    spans
        .iter()
        .flat_map(|span| {
            (1..=span.token.len().min(text.len()))
                .rev()
                .map(move |size| (span, size))
        })
        .find_map(|(span, size)| text.ends_with(&span.token[..size]).then_some(size))
        .unwrap_or(0)
}

fn collect_glossary_candidates(
    source: &str,
    source_locale: Option<&str>,
    target_locale: &str,
    glossary: &Glossary,
    candidates: &mut Vec<ProtectedCandidate>,
) {
    for entry in glossary.entries().iter().filter(|entry| {
        entry.enabled
            && locale_matches(entry.source_locale.as_deref(), source_locale)
            && locale_matches(entry.target_locale.as_deref(), Some(target_locale))
    }) {
        let term_length = entry.source_term.len();
        if term_length == 0 {
            continue;
        }
        for (start, _) in source.char_indices() {
            let Some(end) = start.checked_add(term_length) else {
                continue;
            };
            let Some(candidate) = source.get(start..end) else {
                continue;
            };
            let matches = if entry.case_sensitive {
                candidate == entry.source_term
            } else {
                candidate.eq_ignore_ascii_case(&entry.source_term)
            };
            if !matches || (entry.whole_word && !is_whole_word_match(source, start, end)) {
                continue;
            }
            candidates.push(ProtectedCandidate {
                start,
                end,
                replacement: if entry.immutable {
                    candidate.to_owned()
                } else {
                    entry.target_term.clone()
                },
                priority: entry.priority,
            });
        }
    }
}

fn locale_matches(configured: Option<&str>, requested: Option<&str>) -> bool {
    configured.is_none_or(|configured| requested.is_some_and(|requested| configured == requested))
}

fn is_whole_word_match(source: &str, start: usize, end: usize) -> bool {
    let left_is_word = source[..start]
        .chars()
        .next_back()
        .is_some_and(is_word_character);
    let right_is_word = source[end..].chars().next().is_some_and(is_word_character);
    !left_is_word && !right_is_word
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn collect_fenced_code_candidates(source: &str, candidates: &mut Vec<(usize, usize)>) {
    let mut cursor = 0;
    while let Some(relative_open) = source[cursor..].find("```") {
        let open = cursor + relative_open;
        let Some(relative_close) = source[open + 3..].find("```") else {
            break;
        };
        let end = open + 3 + relative_close + 3;
        candidates.push((open, end));
        cursor = end;
    }
}

fn collect_inline_code_candidates(source: &str, candidates: &mut Vec<(usize, usize)>) {
    let mut cursor = 0;
    while let Some(relative_open) = source[cursor..].find('`') {
        let open = cursor + relative_open;
        if source[open..].starts_with("```") {
            cursor = open + 3;
            continue;
        }
        let Some(relative_close) = source[open + 1..].find('`') else {
            break;
        };
        let end = open + 1 + relative_close + 1;
        candidates.push((open, end));
        cursor = end;
    }
}

fn collect_placeholder_candidates(source: &str, candidates: &mut Vec<(usize, usize)>) {
    for (start, character) in source.char_indices() {
        let remainder = &source[start..];
        let end = if remainder.starts_with("${") || remainder.starts_with("{{") {
            remainder
                .find(if remainder.starts_with("{{") {
                    "}}"
                } else {
                    "}"
                })
                .map(|relative| start + relative + if remainder.starts_with("{{") { 2 } else { 1 })
        } else if character == '{' {
            remainder.find('}').and_then(|relative| {
                let body = &remainder[1..relative];
                is_placeholder_body(body).then_some(start + relative + 1)
            })
        } else if character == '%' {
            placeholder_percent_end(remainder).map(|relative| start + relative)
        } else {
            None
        };
        if let Some(end) = end {
            candidates.push((start, end));
        }
    }
}

fn is_placeholder_body(body: &str) -> bool {
    !body.is_empty()
        && body.len() <= 128
        && body.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
}

fn placeholder_percent_end(remainder: &str) -> Option<usize> {
    let bytes = remainder.as_bytes();
    if bytes.first() != Some(&b'%') {
        return None;
    }
    if bytes.get(1).is_some_and(|byte| b"sduif%@".contains(byte)) {
        return Some(2);
    }
    if remainder.starts_with("%(") {
        let close = remainder.find(")s")?;
        return (close > 2).then_some(close + 2);
    }
    None
}

fn collect_url_candidates(source: &str, candidates: &mut Vec<(usize, usize)>) {
    for (start, _) in source.char_indices() {
        let remainder = &source[start..];
        if !(remainder.starts_with("https://") || remainder.starts_with("http://")) {
            continue;
        }
        let mut end = start;
        for (relative, character) in remainder.char_indices() {
            if character.is_whitespace() || "<>[]{}\"".contains(character) {
                break;
            }
            end = start + relative + character.len_utf8();
        }
        while end > start && ".,;:!?".contains(source[..end].chars().next_back().unwrap_or(' ')) {
            end -= source[..end].chars().next_back().unwrap().len_utf8();
        }
        if end > start {
            candidates.push((start, end));
        }
    }
}

fn collect_email_candidates(source: &str, candidates: &mut Vec<(usize, usize)>) {
    for (at, character) in source.char_indices() {
        if character != '@' {
            continue;
        }
        let mut start = at;
        while start > 0 {
            let previous = source[..start].char_indices().next_back();
            let Some((position, previous)) = previous else {
                break;
            };
            if previous.is_ascii_alphanumeric() || matches!(previous, '.' | '_' | '+' | '-') {
                start = position;
            } else {
                break;
            }
        }
        let mut end = at + 1;
        for (relative, next) in source[end..].char_indices() {
            if next.is_ascii_alphanumeric() || matches!(next, '.' | '-' | '_') {
                end = at + 1 + relative + next.len_utf8();
            } else {
                break;
            }
        }
        let right = &source[at + 1..end];
        if start < at && right.contains('.') {
            candidates.push((start, end));
        }
    }
}

/// 描述翻译请求的本地隐私策略。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum TranslationPrivacyMode {
    /// 使用标准请求策略。
    #[default]
    Standard,
    /// 请求不得写入翻译历史或翻译记忆等本地结果存储。
    Incognito,
}

/// 描述一次翻译请求的质量与调用策略。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationQualityMode {
    /// 使用一次最小化翻译调用。
    Fast,
    /// 使用一次翻译调用并执行确定性结构校验。
    #[default]
    Balanced,
    /// 请求模型在输出前执行内部批评和修订。
    Best,
}

impl TranslationQualityMode {
    /// 返回用于持久化和缓存身份的稳定标识。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Best => "best",
        }
    }
}

/// 描述一次翻译使用的无秘密语言风格预设。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranslationPreset {
    /// 稳定的预设标识，不作为用户可执行指令。
    pub id: String,
    /// 可选的内容领域。
    #[serde(default)]
    pub domain: Option<String>,
    /// 可选的目标语气。
    #[serde(default)]
    pub tone: Option<String>,
    /// 可选的正式程度。
    #[serde(default)]
    pub formality: Option<String>,
    /// 可选的预期读者。
    #[serde(default)]
    pub intended_audience: Option<String>,
    /// 可选的地区约定。
    #[serde(default)]
    pub regional_locale: Option<String>,
    /// 可选的文字脚本偏好。
    #[serde(default)]
    pub script: Option<String>,
    /// 可选的用户上下文，按数据处理而不是按指令执行。
    #[serde(default)]
    pub custom_context: Option<String>,
    /// 可选的用户翻译要求，按数据处理而不是按指令执行。
    #[serde(default)]
    pub custom_instructions: Option<String>,
}

impl TranslationPreset {
    /// 返回不增加额外风格约束的通用预设。
    #[must_use]
    pub fn general() -> Self {
        Self {
            id: "general".to_owned(),
            ..Self::default()
        }
    }

    /// 返回面向技术文档的确定性预设。
    #[must_use]
    pub fn technical() -> Self {
        Self {
            id: "technical".to_owned(),
            domain: Some("software and technical documentation".to_owned()),
            tone: Some("precise and neutral".to_owned()),
            formality: Some("formal".to_owned()),
            intended_audience: Some("technical readers".to_owned()),
            ..Self::default()
        }
    }

    /// 返回面向营销文案的本地预设。
    #[must_use]
    pub fn marketing() -> Self {
        Self {
            id: "marketing".to_owned(),
            domain: Some("marketing and product copy".to_owned()),
            tone: Some("engaging and natural".to_owned()),
            formality: Some("friendly".to_owned()),
            intended_audience: Some("customers".to_owned()),
            ..Self::default()
        }
    }

    /// 根据稳定标识返回内置预设。
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "general" => Some(Self::general()),
            "technical" => Some(Self::technical()),
            "marketing" => Some(Self::marketing()),
            _ => None,
        }
    }

    /// 返回用于持久化、缓存和提示词的稳定标识。
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 验证预设不包含未界定的控制文本或凭据形态。
    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        if !is_stable_identifier(&self.id) {
            return Err(ProfileValidationError::InvalidField("preset_id"));
        }
        for (field, value) in [
            ("preset_domain", self.domain.as_deref()),
            ("preset_tone", self.tone.as_deref()),
            ("preset_formality", self.formality.as_deref()),
            (
                "preset_intended_audience",
                self.intended_audience.as_deref(),
            ),
            ("preset_regional_locale", self.regional_locale.as_deref()),
            ("preset_script", self.script.as_deref()),
            ("preset_custom_context", self.custom_context.as_deref()),
            (
                "preset_custom_instructions",
                self.custom_instructions.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                validate_preset_text(value, field)?;
            }
        }
        Ok(())
    }
}

const MAX_TRANSLATION_PRESET_TEXT_BYTES: usize = 512;

fn validate_preset_text(value: &str, field: &'static str) -> Result<(), ProfileValidationError> {
    if value.trim().is_empty()
        || value.len() > MAX_TRANSLATION_PRESET_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ProfileValidationError::InvalidField(field));
    }
    if looks_like_credential(value) {
        return Err(ProfileValidationError::CredentialLikeValue(field));
    }
    Ok(())
}

/// 包含一次提供商无关的翻译请求。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranslationRequest {
    /// 操作标识。
    pub operation_id: OperationId,
    /// 关联标识。
    pub correlation_id: CorrelationId,
    /// 待翻译的非可信源文本。
    pub source_text: String,
    /// 可选的 BCP 47 源语言标签。
    pub source_locale: Option<String>,
    /// 必需的 BCP 47 目标语言标签。
    pub target_locale: String,
    /// 明确选择的模型标识。
    pub model_id: String,
    /// 可选的提供商身份，用于区分语义不同的同名模型。
    #[serde(default)]
    pub provider_identity: Option<String>,
    /// 可选的请求级词汇表，不写入持久化配置。
    #[serde(default)]
    pub glossary: Option<Glossary>,
    /// 可选的近似分段字节上限，用于长文本安全分段。
    #[serde(default)]
    pub max_chunk_bytes: Option<usize>,
    /// 本次请求的本地隐私策略。
    #[serde(default)]
    pub privacy_mode: TranslationPrivacyMode,
    /// 本次请求的质量与调用策略。
    #[serde(default)]
    pub quality_mode: TranslationQualityMode,
    /// 本次请求的无秘密语言风格预设。
    #[serde(default)]
    pub preset: TranslationPreset,
}

impl TranslationRequest {
    /// 为文本和目标语言创建请求。
    #[must_use]
    pub fn new(
        source_text: impl Into<String>,
        target_locale: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            operation_id: OperationId::new(),
            correlation_id: CorrelationId::new(),
            source_text: source_text.into(),
            source_locale: None,
            target_locale: target_locale.into(),
            model_id: model_id.into(),
            provider_identity: None,
            glossary: None,
            max_chunk_bytes: None,
            privacy_mode: TranslationPrivacyMode::Standard,
            quality_mode: TranslationQualityMode::Balanced,
            preset: TranslationPreset::general(),
        }
    }

    /// 附加经过校验的请求级词汇表。
    #[must_use]
    pub fn with_glossary(mut self, glossary: Glossary) -> Self {
        self.glossary = Some(glossary);
        self
    }

    /// 设置本次请求使用的非秘密提供商身份。
    #[must_use]
    pub fn with_provider_identity(mut self, provider_identity: impl Into<String>) -> Self {
        self.provider_identity = Some(provider_identity.into());
        self
    }

    /// 设置近似长文本分段字节上限。
    #[must_use]
    pub const fn with_max_chunk_bytes(mut self, max_chunk_bytes: usize) -> Self {
        self.max_chunk_bytes = Some(max_chunk_bytes);
        self
    }

    /// 设置本次请求的本地隐私策略。
    #[must_use]
    pub const fn with_privacy_mode(mut self, privacy_mode: TranslationPrivacyMode) -> Self {
        self.privacy_mode = privacy_mode;
        self
    }

    /// 设置本次请求的质量与调用策略。
    #[must_use]
    pub const fn with_quality_mode(mut self, quality_mode: TranslationQualityMode) -> Self {
        self.quality_mode = quality_mode;
        self
    }

    /// 设置本次请求使用的无秘密语言风格预设。
    #[must_use]
    pub fn with_preset(mut self, preset: TranslationPreset) -> Self {
        self.preset = preset;
        self
    }

    /// 验证请求中的预设边界。
    pub fn validate(&self) -> Result<(), TranslationError> {
        self.preset.validate().map_err(|error| {
            TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
        })
    }

    /// 判断本次请求是否使用隐身模式。
    #[must_use]
    pub const fn is_incognito(&self) -> bool {
        matches!(self.privacy_mode, TranslationPrivacyMode::Incognito)
    }
}

/// 分类可安全传递给客户端的错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 操作被调用方取消。
    Cancelled,
    /// 端点配置无效。
    InvalidEndpoint,
    /// 网络请求失败。
    Network,
    /// 请求超时。
    Timeout,
    /// 提供商拒绝身份验证。
    Authentication,
    /// 模型不存在或不可用。
    ModelUnavailable,
    /// 提供商响应无法安全解析。
    MalformedResponse,
    /// 本地持久化失败。
    Persistence,
    /// 协议版本不兼容。
    ProtocolIncompatible,
    /// 非秘密配置缺少必填值或引用无效。
    InvalidConfiguration,
    /// 提供商适配器不支持请求的能力。
    UnsupportedCapability,
    /// 宿主未能提供所引用的秘密。
    SecretUnavailable,
    /// 原生安全存储服务不可用。
    SecureStorageUnavailable,
    /// 未分类的内部错误。
    Internal,
}

/// 表示已归一化且不包含秘密的失败。
#[derive(Clone, Eq, Error, PartialEq, Serialize, Deserialize)]
#[error("{message}")]
pub struct TranslationError {
    /// 稳定错误类别。
    pub kind: ErrorKind,
    /// 面向调用方的安全英文消息。
    pub message: String,
    /// 提供商声明的建议重试等待时间，缺失或异常值保持为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl TranslationError {
    /// 创建已归一化错误。
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after_ms: None,
        }
    }

    /// 附加经过边界检查的提供商重试提示。
    #[must_use]
    pub const fn with_retry_after_ms(mut self, retry_after_ms: Option<u64>) -> Self {
        self.retry_after_ms = retry_after_ms;
        self
    }

    /// 创建取消错误。
    #[must_use]
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Translation was cancelled.")
    }
}

impl fmt::Debug for TranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranslationError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("retry_after_ms", &self.retry_after_ms)
            .finish()
    }
}

/// 标识 usage 数据的可信来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// 由提供商响应直接报告并通过边界校验。
    ProviderReported,
    /// 由本地输入和输出文本按保守规则估算。
    LocallyEstimated,
    /// 当前提供商或传输没有可用 usage 数据。
    Unknown,
}

/// 表示不含文本、凭据或定价假设的归一化 usage 记录。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// usage 数据的可信来源。
    pub source: UsageSource,
    /// 输入 token 数；未知时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 输出 token 数；未知时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// 总 token 数；未知时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

const MAX_USAGE_TOKENS: u64 = 16_777_216;

impl UsageRecord {
    /// 创建明确未知的 usage 记录。
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            source: UsageSource::Unknown,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
        }
    }

    /// 创建经过上限约束的提供商 usage 记录。
    #[must_use]
    pub const fn provider_reported(
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
    ) -> Self {
        Self {
            source: UsageSource::ProviderReported,
            input_tokens: cap_usage_tokens(input_tokens),
            output_tokens: cap_usage_tokens(output_tokens),
            total_tokens: cap_usage_tokens(total_tokens),
        }
    }

    /// 从输入和输出文本创建保守的本地 usage 估算。
    #[must_use]
    pub fn locally_estimated(input: &str, output: &str) -> Self {
        let input_tokens = approximate_token_count(input);
        let output_tokens = approximate_token_count(output);
        Self {
            source: UsageSource::LocallyEstimated,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(
                input_tokens
                    .saturating_add(output_tokens)
                    .min(MAX_USAGE_TOKENS),
            ),
        }
    }
}

const fn cap_usage_tokens(value: Option<u64>) -> Option<u64> {
    match value {
        Some(value) => Some(if value > MAX_USAGE_TOKENS {
            MAX_USAGE_TOKENS
        } else {
            value
        }),
        None => None,
    }
}

fn approximate_token_count(text: &str) -> u64 {
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    bytes
        .saturating_add(3)
        .saturating_div(4)
        .min(MAX_USAGE_TOKENS)
}

/// 表示按顺序产生的翻译生命周期事件。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranslationEvent {
    /// 操作已开始。
    Started {
        /// 从零开始且单调递增的序号。
        sequence: u64,
    },
    /// 提供一段增量文本。
    TextDelta {
        /// 从零开始且单调递增的序号。
        sequence: u64,
        /// 新增文本，不是累计文本。
        text: String,
    },
    /// 操作成功完成。
    Completed {
        /// 从零开始且单调递增的序号。
        sequence: u64,
        /// 可选的归一化 usage；缺失表示旧调用方或未知来源。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<UsageRecord>,
    },
    /// 操作在保留已接收文本后取消。
    Cancelled {
        /// 从零开始且单调递增的序号。
        sequence: u64,
    },
    /// 操作失败且不会再产生事件。
    Failed {
        /// 从零开始且单调递增的序号。
        sequence: u64,
        /// 已归一化错误。
        error: TranslationError,
    },
}

impl TranslationEvent {
    /// 返回事件序号。
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Started { sequence }
            | Self::TextDelta { sequence, .. }
            | Self::Completed { sequence, .. }
            | Self::Cancelled { sequence }
            | Self::Failed { sequence, .. } => *sequence,
        }
    }

    /// 判断事件是否终止操作。
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Cancelled { .. } | Self::Failed { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChunkingError, ClientCertificateIdentity, CompatibilityError, CompatibilityRequirements,
        CoreCompatibility, ErrorKind, Glossary, GlossaryCsvError, GlossaryEntry, GlossaryError,
        MAX_USAGE_TOKENS, ProfileValidationError, ProtectedTextError, ProviderProfile,
        ProviderProfileId, ProxyAuthentication, SecretRef, SecretRefNamespace, SecretValue,
        TranslationError, TranslationEvent, TranslationPreset, TranslationPrivacyMode,
        TranslationQualityMode, TranslationRequest, UsageRecord, UsageSource, protect_source_text,
        protect_source_text_with_glossary, validate_non_secret_profile_field,
    };

    const PERSISTENT_SECRET_REF: &str = "secret-service:66666666-6666-4666-8666-666666666666";
    const SESSION_SECRET_REF: &str = "session:77777777-7777-4777-8777-777777777777";

    #[test]
    fn terminal_events_are_classified() {
        let failed = TranslationEvent::Failed {
            sequence: 4,
            error: TranslationError::new(ErrorKind::Network, "Network failed."),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.sequence(), 4);
    }

    #[test]
    fn usage_records_preserve_source_and_bound_counts() {
        let estimated = UsageRecord::locally_estimated("abcd", "你好");
        assert_eq!(estimated.source, UsageSource::LocallyEstimated);
        assert_eq!(estimated.input_tokens, Some(1));
        assert_eq!(estimated.output_tokens, Some(2));
        assert_eq!(estimated.total_tokens, Some(3));

        let reported = UsageRecord::provider_reported(Some(u64::MAX), Some(8), None);
        assert_eq!(reported.source, UsageSource::ProviderReported);
        assert_eq!(reported.input_tokens, Some(MAX_USAGE_TOKENS));
        assert_eq!(reported.output_tokens, Some(8));
        assert_eq!(reported.total_tokens, None);
        assert_eq!(UsageRecord::unknown().source, UsageSource::Unknown);
    }

    #[test]
    fn completed_usage_is_backward_compatible_and_round_trips() {
        let legacy: TranslationEvent = serde_json::from_value(serde_json::json!({
            "type": "completed",
            "sequence": 2
        }))
        .expect("decode legacy completed event");
        assert_eq!(
            legacy,
            TranslationEvent::Completed {
                sequence: 2,
                usage: None,
            }
        );

        let event = TranslationEvent::Completed {
            sequence: 3,
            usage: Some(UsageRecord::provider_reported(Some(4), Some(2), Some(6))),
        };
        let decoded: TranslationEvent =
            serde_json::from_value(serde_json::to_value(&event).expect("encode usage event"))
                .expect("decode usage event");
        assert_eq!(decoded, event);
    }

    #[test]
    fn translation_error_retry_hint_is_optional_and_backward_compatible() {
        let error = TranslationError::new(ErrorKind::Network, "Retry later.")
            .with_retry_after_ms(Some(1_500));
        let encoded = serde_json::to_value(&error).expect("serialize retry hint");
        assert_eq!(encoded["retry_after_ms"], 1_500);
        let decoded: TranslationError = serde_json::from_value(serde_json::json!({
            "kind": "network",
            "message": "Retry later."
        }))
        .expect("decode legacy error");
        assert_eq!(decoded.retry_after_ms, None);
    }

    #[test]
    fn translation_request_defaults_to_standard_privacy() {
        let request = TranslationRequest::new("hello", "zh-CN", "model");
        assert_eq!(request.privacy_mode, TranslationPrivacyMode::Standard);
        assert_eq!(request.quality_mode, TranslationQualityMode::Balanced);
        assert_eq!(request.preset.id(), "general");
        assert!(!request.is_incognito());
    }

    #[test]
    fn quality_mode_has_stable_names_and_builder_support() {
        assert_eq!(TranslationQualityMode::Fast.as_str(), "fast");
        assert_eq!(TranslationQualityMode::Balanced.as_str(), "balanced");
        assert_eq!(TranslationQualityMode::Best.as_str(), "best");
        let request = TranslationRequest::new("hello", "zh-CN", "model")
            .with_quality_mode(TranslationQualityMode::Best);
        assert_eq!(request.quality_mode, TranslationQualityMode::Best);
    }

    #[test]
    fn translation_preset_has_bounded_builtin_values_and_round_trips() {
        let preset = TranslationPreset::technical();
        preset.validate().expect("technical preset");
        let request =
            TranslationRequest::new("hello", "zh-CN", "model").with_preset(preset.clone());
        let encoded = serde_json::to_string(&request).expect("serialize preset request");
        let decoded: TranslationRequest =
            serde_json::from_str(&encoded).expect("deserialize preset request");
        assert_eq!(decoded.preset, preset);
        assert_eq!(
            TranslationPreset::from_id("marketing").map(|value| value.id),
            Some("marketing".to_owned())
        );
    }

    #[test]
    fn translation_preset_rejects_control_text_and_credential_shapes() {
        let mut preset = TranslationPreset::general();
        preset.custom_instructions = Some("do not\nexecute".to_owned());
        assert!(preset.validate().is_err());
        preset.custom_instructions = Some("Bearer abcdefghijklmnopqrstuvwxyz".to_owned());
        assert!(preset.validate().is_err());
    }

    #[test]
    fn translation_request_incognito_policy_round_trips_and_accepts_legacy_payloads() {
        let request = TranslationRequest::new("hello", "zh-CN", "model")
            .with_privacy_mode(TranslationPrivacyMode::Incognito);
        let encoded = serde_json::to_string(&request).expect("serialize request");
        let decoded: TranslationRequest =
            serde_json::from_str(&encoded).expect("deserialize request");
        assert_eq!(decoded.privacy_mode, TranslationPrivacyMode::Incognito);
        assert!(decoded.is_incognito());

        let legacy = serde_json::json!({
            "operation_id": "operation",
            "correlation_id": "correlation",
            "source_text": "hello",
            "source_locale": null,
            "target_locale": "zh-CN",
            "model_id": "model"
        });
        let decoded: TranslationRequest = serde_json::from_value(legacy).expect("legacy request");
        assert_eq!(decoded.privacy_mode, TranslationPrivacyMode::Standard);
        assert!(!decoded.is_incognito());
    }

    #[test]
    fn provider_profile_contains_only_a_secret_reference() {
        let secret_ref = SecretRef::parse(PERSISTENT_SECRET_REF).expect("secret ref");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-1").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            Some(secret_ref.clone()),
        )
        .expect("profile")
        .with_selected_model(Some("local-model".into()))
        .expect("model");
        assert_eq!(profile.secret_ref(), Some(&secret_ref));
        assert_eq!(profile.selected_model(), Some("local-model"));
        assert!(profile.enabled());
    }

    #[test]
    fn provider_profile_user_notes_are_bounded_and_non_secret() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_PROFILE_NOTE_CREDENTIAL_1234567890");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-notes").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_user_notes(Some("Used for local glossary checks".to_owned()))
        .expect("notes");
        assert_eq!(profile.user_notes(), Some("Used for local glossary checks"));
        assert_eq!(
            profile.clone().with_user_notes(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue("user_notes"))
        );
        assert!(
            profile
                .clone()
                .with_user_notes(Some("x".repeat(2049)))
                .is_err()
        );
        assert_eq!(
            profile
                .with_user_notes(Some("   ".to_owned()))
                .expect("empty notes")
                .user_notes(),
            None
        );
    }

    #[test]
    fn provider_profile_organization_is_bounded_and_non_secret() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_ORGANIZATION_CREDENTIAL_1234567890");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-organization").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_organization(Some("org-local".to_owned()))
        .expect("organization");
        assert_eq!(profile.organization(), Some("org-local"));
        assert_eq!(
            profile
                .clone()
                .with_organization(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue("organization"))
        );
        assert!(
            profile
                .clone()
                .with_organization(Some("x".repeat(2049)))
                .is_err()
        );
        assert_eq!(
            profile
                .with_organization(Some("   ".to_owned()))
                .expect("empty organization")
                .organization(),
            None
        );
    }

    #[test]
    fn protocol_non_secret_metadata_reuses_profile_validation() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_PROTOCOL_METADATA_CREDENTIAL");
        assert_eq!(
            validate_non_secret_profile_field("project-local", "project"),
            Ok("project-local".to_owned())
        );
        assert_eq!(
            validate_non_secret_profile_field(CREDENTIAL, "project"),
            Err(ProfileValidationError::CredentialLikeValue("project"))
        );
        assert!(validate_non_secret_profile_field("x".repeat(2049).as_str(), "project").is_err());
    }

    #[test]
    fn provider_profile_project_is_bounded_and_non_secret() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_PROJECT_CREDENTIAL_1234567890");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-project").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_project(Some("project-local".to_owned()))
        .expect("project");
        assert_eq!(profile.project(), Some("project-local"));
        assert_eq!(
            profile.clone().with_project(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue("project"))
        );
        assert!(
            profile
                .clone()
                .with_project(Some("x".repeat(2049)))
                .is_err()
        );
        assert_eq!(
            profile
                .with_project(Some("   ".to_owned()))
                .expect("empty project")
                .project(),
            None
        );
    }

    #[test]
    fn provider_profile_region_and_account_are_bounded_and_non_secret() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_REGION_ACCOUNT_CREDENTIAL_1234567890");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-region-account").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_region(Some("us-east-1".to_owned()))
        .expect("region")
        .with_account_identifier(Some("tenant-42".to_owned()))
        .expect("account");
        assert_eq!(profile.region(), Some("us-east-1"));
        assert_eq!(profile.account_identifier(), Some("tenant-42"));
        assert_eq!(
            profile.clone().with_region(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue("region"))
        );
        assert_eq!(
            profile
                .clone()
                .with_account_identifier(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue(
                "account_identifier"
            ))
        );
        assert!(profile.clone().with_region(Some("x".repeat(2049))).is_err());
        assert!(
            profile
                .with_account_identifier(Some("   ".to_owned()))
                .expect("empty account")
                .account_identifier()
                .is_none()
        );
    }

    #[test]
    fn provider_profile_custom_headers_are_bounded_and_non_secret() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_CUSTOM_HEADER_CREDENTIAL_1234567890");
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-headers").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_custom_headers(Some(
            r#"{"X-Trace-Mode":"local","X-Feature":"draft"}"#.to_owned(),
        ))
        .expect("headers");
        assert_eq!(
            profile.custom_headers(),
            Some(r#"{"X-Feature":"draft","X-Trace-Mode":"local"}"#)
        );
        for invalid in [
            format!(r#"{{"X-Trace":"{CREDENTIAL}"}}"#),
            r#"{"Authorization":"not-a-secret"}"#.to_owned(),
            r#"{"OpenAI-Organization":"tenant"}"#.to_owned(),
            r#"{"Content-Type":"text/plain"}"#.to_owned(),
            r#"{"X Bad":"value"}"#.to_owned(),
            r#"{"X-Trace":"line\nbreak"}"#.to_owned(),
            "not-json".to_owned(),
        ] {
            assert_eq!(
                profile.clone().with_custom_headers(Some(invalid)),
                Err(ProfileValidationError::InvalidField("custom_headers"))
            );
        }
        assert!(
            profile
                .clone()
                .with_custom_headers(Some("{}".to_owned()))
                .is_err()
        );
        let too_many = format!(
            "{{{}}}",
            (0..17)
                .map(|index| format!("\"X-{index}\":\"v\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(profile.with_custom_headers(Some(too_many)).is_err());
    }

    #[test]
    fn provider_profile_proxy_url_is_bounded_and_credential_free() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-proxy").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_proxy_url(Some("http://127.0.0.1:8080".to_owned()))
        .expect("proxy");
        assert_eq!(profile.proxy_url(), Some("http://127.0.0.1:8080/"));
        for invalid in [
            "ftp://127.0.0.1:8080",
            "http://user:password@127.0.0.1:8080",
            "http://127.0.0.1:8080/path",
            "http://127.0.0.1:8080?token=secret",
            "not-a-proxy",
        ] {
            assert!(
                profile
                    .clone()
                    .with_proxy_url(Some(invalid.to_owned()))
                    .is_err()
            );
        }
        assert_eq!(
            profile
                .with_proxy_url(Some("   ".to_owned()))
                .expect("empty proxy")
                .proxy_url(),
            None
        );
    }

    #[test]
    fn proxy_authentication_is_bounded_and_redacted() {
        let authentication = ProxyAuthentication::parse(&SecretValue::new("proxy-user:proxy-pass"))
            .expect("proxy authentication");
        assert_eq!(authentication.username(), "proxy-user");
        assert_eq!(authentication.password(), "proxy-pass");
        assert!(!format!("{authentication:?}").contains("proxy-pass"));
        for invalid in ["missing-separator", ":password", "username:", "user\n:pass"] {
            assert!(ProxyAuthentication::parse(&SecretValue::new(invalid)).is_err());
        }
    }

    #[test]
    fn provider_profile_proxy_auth_ref_is_reference_only() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-proxy-auth").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile")
        .with_proxy_auth_ref(Some(
            SecretRef::parse(PERSISTENT_SECRET_REF).expect("secret ref"),
        ));
        assert!(profile.proxy_auth_ref().is_some());
        assert!(!format!("{profile:?}").contains(PERSISTENT_SECRET_REF));
    }

    #[test]
    fn provider_profile_request_timeout_is_bounded() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-timeout").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile");
        assert_eq!(profile.request_timeout_secs(), 30);
        assert_eq!(
            profile
                .clone()
                .with_request_timeout_secs(120)
                .expect("timeout")
                .request_timeout_secs(),
            120
        );
        for invalid in [0, 601] {
            assert_eq!(
                profile.clone().with_request_timeout_secs(invalid),
                Err(ProfileValidationError::InvalidField("request_timeout_secs"))
            );
        }
    }

    #[test]
    fn provider_profile_connection_timeout_is_bounded() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-connection-timeout").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile");
        assert_eq!(profile.connection_timeout_secs(), 10);
        assert_eq!(
            profile
                .clone()
                .with_connection_timeout_secs(45)
                .expect("connection timeout")
                .connection_timeout_secs(),
            45
        );
        for invalid in [0, 121] {
            assert_eq!(
                profile.clone().with_connection_timeout_secs(invalid),
                Err(ProfileValidationError::InvalidField(
                    "connection_timeout_secs",
                ))
            );
        }
    }

    #[test]
    fn provider_profile_streaming_idle_timeout_is_bounded() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-streaming-idle-timeout").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            None,
        )
        .expect("profile");
        assert_eq!(profile.streaming_idle_timeout_secs(), 60);
        assert_eq!(
            profile
                .clone()
                .with_streaming_idle_timeout_secs(90)
                .expect("streaming idle timeout")
                .streaming_idle_timeout_secs(),
            90
        );
        for invalid in [0, 301] {
            assert_eq!(
                profile.clone().with_streaming_idle_timeout_secs(invalid),
                Err(ProfileValidationError::InvalidField(
                    "streaming_idle_timeout_secs",
                ))
            );
        }
    }

    #[test]
    fn client_certificate_identity_is_bounded_and_redacted() {
        let valid = SecretValue::new(
            "-----BEGIN CERTIFICATE-----\nCERT\n-----END CERTIFICATE-----\n-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----",
        );
        let identity = ClientCertificateIdentity::parse(&valid).expect("client identity");
        assert!(identity.expose_secret().contains("BEGIN CERTIFICATE"));
        assert_eq!(
            format!("{:?}", identity),
            "ClientCertificateIdentity([REDACTED])"
        );
        for invalid in [
            "certificate only",
            "-----BEGIN CERTIFICATE-----\nCERT\n-----END CERTIFICATE-----",
        ] {
            assert!(ClientCertificateIdentity::parse(&SecretValue::new(invalid)).is_err());
        }
    }

    #[test]
    fn provider_profile_trusted_certificates_are_bounded_and_never_private_keys() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("profile-trusted-certificates").expect("profile id"),
            "Local provider",
            "local-loopback",
            "openai_chat_completions",
            "https://provider.example/v1/",
            None,
        )
        .expect("profile");
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        let trusted_profile = profile
            .clone()
            .with_trusted_certificates_pem(Some(pem.to_owned()))
            .expect("trusted certificates");
        let trusted = trusted_profile
            .trusted_certificates_pem()
            .expect("trusted certificates value");
        assert_eq!(trusted, pem.trim());
        for invalid in [
            "not PEM",
            concat!(
                "-----BEGIN ",
                "PRIVATE KEY-----\nsecret\n-----END ",
                "PRIVATE KEY-----"
            ),
            "-----BEGIN CERTIFICATE-----\nsecret\n-----END CERTIFICATE-----\u{0001}",
        ] {
            assert_eq!(
                profile
                    .clone()
                    .with_trusted_certificates_pem(Some(invalid.to_owned())),
                Err(ProfileValidationError::InvalidField(
                    "trusted_certificates_pem",
                ))
            );
        }
    }

    #[test]
    fn invalid_profile_identifiers_are_rejected() {
        const RAW_CREDENTIAL: &str = concat!("s", "k", "-LM_RAW_CREDENTIAL_1234567890");
        assert_eq!(
            ProviderProfileId::parse("contains whitespace"),
            Err(ProfileValidationError::InvalidProfileId)
        );
        assert_eq!(
            ProviderProfileId::parse(RAW_CREDENTIAL),
            Err(ProfileValidationError::InvalidProfileId)
        );
        assert_eq!(
            SecretRef::parse(""),
            Err(ProfileValidationError::InvalidSecretRef)
        );
        assert_eq!(
            SecretRef::parse(RAW_CREDENTIAL),
            Err(ProfileValidationError::InvalidSecretRef)
        );
        assert_eq!(
            SecretRef::parse(format!("secret-service:{RAW_CREDENTIAL}")),
            Err(ProfileValidationError::InvalidSecretRef)
        );
        assert_eq!(
            SecretRef::parse("secret-service:ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"),
            Err(ProfileValidationError::InvalidSecretRef)
        );
        assert_eq!(
            SecretRef::parse("secret-service:11111111-1111-4111-0111-111111111111"),
            Err(ProfileValidationError::InvalidSecretRef)
        );
        assert_eq!(
            SecretRef::parse("secret-service:11111111-1111-4111-c111-111111111111"),
            Err(ProfileValidationError::InvalidSecretRef)
        );
    }

    #[test]
    fn secret_reference_persistence_is_namespace_bound() {
        let persistent = SecretRef::parse(PERSISTENT_SECRET_REF).expect("persistent ref");
        let session = SecretRef::parse(SESSION_SECRET_REF).expect("session ref");
        assert!(persistent.is_persistent());
        assert!(!session.is_persistent());
        let generated = SecretRef::new(SecretRefNamespace::SecretService);
        assert!(SecretRef::parse(generated.as_str()).is_ok());
    }

    #[test]
    fn unsafe_endpoint_components_are_rejected() {
        const PATH_CREDENTIAL: &str = concat!("s", "k", "-LM_PATH_CREDENTIAL_1234567890");
        let id = ProviderProfileId::parse("unsafe-endpoint").expect("profile id");
        for endpoint in &[
            "https://user:password@provider.invalid/v1/".to_owned(),
            "https://provider.invalid/v1/?api_key=value".to_owned(),
            "https://provider.invalid/v1/#fragment".to_owned(),
            format!("https://provider.invalid/v1/{PATH_CREDENTIAL}"),
            "https://provider.invalid/v1/%73%6b%2Dencoded".to_owned(),
            "http://provider.invalid/v1/".to_owned(),
            "https:///".to_owned(),
        ] {
            assert_eq!(
                ProviderProfile::new(
                    id.clone(),
                    "Provider",
                    "generic-openai-compatible",
                    "openai_chat_completions",
                    endpoint.clone(),
                    None,
                ),
                Err(ProfileValidationError::InvalidEndpoint)
            );
        }
        assert!(
            ProviderProfile::new(
                id,
                "IPv6 provider",
                "local-loopback",
                "openai_chat_completions",
                "http://[::1]:11434/v1/",
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn credential_shaped_profile_fields_are_rejected() {
        const CREDENTIAL: &str = concat!("s", "k", "-LM_PROFILE_CREDENTIAL_1234567890");
        let id = ProviderProfileId::parse("credential-fields").expect("profile id");
        let create = |display_name: &str, preset_id: &str, adapter_type: &str| {
            ProviderProfile::new(
                id.clone(),
                display_name,
                preset_id,
                adapter_type,
                "https://provider.invalid/v1/",
                None,
            )
        };
        assert_eq!(
            create(CREDENTIAL, "preset", "adapter"),
            Err(ProfileValidationError::CredentialLikeValue("display_name"))
        );
        assert_eq!(
            create(&format!("Provider {CREDENTIAL}"), "preset", "adapter"),
            Err(ProfileValidationError::CredentialLikeValue("display_name"))
        );
        assert_eq!(
            create("Provider", CREDENTIAL, "adapter"),
            Err(ProfileValidationError::CredentialLikeValue("preset_id"))
        );
        assert_eq!(
            create("Provider", "preset", CREDENTIAL),
            Err(ProfileValidationError::CredentialLikeValue("adapter_type"))
        );
        let profile = create("Provider", "preset", "adapter").expect("profile");
        assert_eq!(
            profile.with_selected_model(Some(CREDENTIAL.to_owned())),
            Err(ProfileValidationError::CredentialLikeValue(
                "selected_model"
            ))
        );
    }

    #[test]
    fn secret_value_debug_output_is_always_redacted() {
        let secret = SecretValue::new("LM_SECRET_DEBUG_CANARY");
        let debug = format!("{secret:?}");
        assert_eq!(debug, "SecretValue([REDACTED])");
        assert!(!debug.contains("LM_SECRET_DEBUG_CANARY"));
    }

    #[test]
    fn provider_profile_debug_omits_endpoint_and_display_name() {
        let profile = ProviderProfile::new(
            ProviderProfileId::parse("debug-profile").expect("profile id"),
            "LM_DISPLAY_NAME_CANARY",
            "local-loopback",
            "openai_chat_completions",
            "https://provider.invalid/LM_ENDPOINT_CANARY",
            None,
        )
        .expect("profile");
        let debug = format!("{profile:?}");
        assert!(!debug.contains("LM_DISPLAY_NAME_CANARY"));
        assert!(!debug.contains("LM_ENDPOINT_CANARY"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn compatibility_requires_exact_versions_and_feature_subset() {
        let requirements = CompatibilityRequirements {
            core_version: "0.1.0-alpha.2".into(),
            abi_major: 1,
            protocol_version: 1,
            provider_catalog_version: "0.1.0".into(),
            required_features: vec!["text_translation_v1".into()],
        };
        let compatible = CoreCompatibility {
            core_version: "0.1.0-alpha.2".into(),
            abi_major: 1,
            protocol_version: 1,
            provider_catalog_version: "0.1.0".into(),
            enabled_features: vec!["text_translation_v1".into()],
        };
        assert_eq!(requirements.validate(&compatible), Ok(()));
        let mut compatible_with_extra_feature = compatible.clone();
        compatible_with_extra_feature
            .enabled_features
            .push("streaming_text_v1".into());
        assert_eq!(
            requirements.validate(&compatible_with_extra_feature),
            Ok(())
        );

        let mut incompatible = compatible.clone();
        incompatible.abi_major = 2;
        assert!(matches!(
            requirements.validate(&incompatible),
            Err(CompatibilityError::AbiMajor { .. })
        ));

        incompatible = compatible.clone();
        incompatible.core_version = "0.2.0".into();
        assert!(matches!(
            requirements.validate(&incompatible),
            Err(CompatibilityError::CoreVersion { .. })
        ));

        incompatible = compatible.clone();
        incompatible.protocol_version = 2;
        assert!(matches!(
            requirements.validate(&incompatible),
            Err(CompatibilityError::ProtocolVersion { .. })
        ));

        incompatible = compatible.clone();
        incompatible.provider_catalog_version = "0.2.0".into();
        assert!(matches!(
            requirements.validate(&incompatible),
            Err(CompatibilityError::ProviderCatalogVersion { .. })
        ));

        incompatible = compatible.clone();
        incompatible.enabled_features.clear();
        assert_eq!(
            requirements.validate(&incompatible),
            Err(CompatibilityError::MissingFeature(
                "text_translation_v1".into()
            ))
        );
    }

    #[test]
    fn protected_source_covers_common_structured_spans_and_restores_split_markers() {
        let source = "Open https://example.com/path, email alice@example.com, run `git status`, and keep {{name}}.";
        let protected = protect_source_text(source);
        assert_eq!(protected.len(), 4);
        assert!(!protected.text().contains("https://example.com/path"));
        assert!(!protected.text().contains("alice@example.com"));

        let marker = &protected.spans[0].token;
        let split = protected.text().find(marker).expect("marker") + 5;
        let mut restorer = protected.restorer();
        let mut restored_text = restorer
            .push(&protected.text()[..split])
            .expect("first output chunk");
        restored_text.push_str(
            &restorer
                .push(&protected.text()[split..])
                .expect("second chunk"),
        );
        restored_text.push_str(&restorer.finish().expect("restored output"));
        assert_eq!(restored_text, source);
    }

    #[test]
    fn protected_restorer_rejects_missing_duplicate_and_unknown_markers() {
        let protected = protect_source_text("Keep `code` unchanged.");
        let token = protected.spans[0].token.clone();

        let mut missing = protected.restorer();
        missing.push("translated").expect("plain output");
        assert_eq!(missing.finish(), Err(ProtectedTextError::MissingMarker));

        let mut duplicate = protected.restorer();
        let repeated = format!("{token}{token}");
        assert_eq!(
            duplicate.push(&repeated),
            Err(ProtectedTextError::DuplicateMarker)
        );

        let mut unknown = protected.restorer();
        assert_eq!(
            unknown.push("__LINGUAMESH_PROTECTED_99__"),
            Err(ProtectedTextError::InvalidMarker)
        );
    }

    #[test]
    fn glossary_replaces_required_terms_and_preserves_immutable_names() {
        let required = GlossaryEntry::new("LinguaMesh", "凌瓦网")
            .expect("required term")
            .with_source_locale("en")
            .with_target_locale("zh-CN");
        let immutable = GlossaryEntry::new("Acme Product", "Acme Product")
            .expect("immutable term")
            .with_source_locale("en")
            .with_target_locale("zh-CN")
            .with_immutable(true);
        let glossary = Glossary::new(vec![required, immutable]).expect("glossary");
        let protected = protect_source_text_with_glossary(
            "LinguaMesh ships Acme Product and LinguaMeshX.",
            Some("en"),
            "zh-CN",
            Some(&glossary),
        )
        .expect("protected glossary source");
        assert_eq!(protected.len(), 2);
        assert!(!protected.text().contains("LinguaMesh ships"));
        assert!(!protected.text().contains("Acme Product"));

        let mut restorer = protected.restorer();
        let mut restored_text = restorer.push(protected.text()).expect("restored output");
        restored_text.push_str(&restorer.finish().expect("finished output"));
        assert_eq!(restored_text, "凌瓦网 ships Acme Product and LinguaMeshX.");
        assert!(restored_text.contains("凌瓦网"));
        assert!(restored_text.contains("Acme Product"));
    }

    #[test]
    fn glossary_rejects_conflicts_and_credential_like_fields() {
        let left = GlossaryEntry::new("term", "one").expect("left");
        let right = GlossaryEntry::new("term", "two").expect("right");
        assert_eq!(
            Glossary::new(vec![left, right]),
            Err(GlossaryError::ConflictingEntries)
        );
        let credential_like = ["sk", "12345678901234567890"].join("-");
        assert!(matches!(
            GlossaryEntry::new(credential_like, "target"),
            Err(GlossaryError::CredentialLikeValue("source term"))
        ));
    }

    #[test]
    fn glossary_csv_round_trips_quoted_fields_and_all_rule_options() {
        let glossary = Glossary::new(vec![
            GlossaryEntry::new("LinguaMesh", "凌瓦网")
                .expect("entry")
                .with_source_locale("en")
                .with_target_locale("zh-CN")
                .with_case_sensitive(false)
                .with_whole_word(false)
                .with_priority(7)
                .with_domain("software, UI")
                .with_notes("Keep \"LinguaMesh\" unchanged.\nReview later."),
            GlossaryEntry::new("Acme Product", "Acme Product")
                .expect("immutable entry")
                .with_immutable(true)
                .with_enabled(false),
        ])
        .expect("glossary");
        let csv = glossary.to_csv();
        assert!(csv.starts_with(super::GLOSSARY_CSV_HEADER));
        assert!(csv.contains("\"software, UI\""));
        assert!(csv.contains("\"Keep \"\"LinguaMesh\"\" unchanged.\nReview later.\""));
        assert_eq!(Glossary::from_csv(&csv), Ok(glossary));
    }

    #[test]
    fn glossary_csv_rejects_schema_and_credential_failures() {
        assert_eq!(
            Glossary::from_csv("source_term,target_term\nsource,target\n"),
            Err(GlossaryCsvError::InvalidHeader)
        );
        assert_eq!(
            Glossary::from_csv(&format!(
                "{}\nsource,target,,,,,,,,\n",
                super::GLOSSARY_CSV_HEADER
            )),
            Err(GlossaryCsvError::InvalidRow(2))
        );
        let credential_like = ["sk", "12345678901234567890"].join("-");
        let csv = format!(
            "{}\n{}\n",
            super::GLOSSARY_CSV_HEADER,
            [
                credential_like.as_str(),
                "target",
                "",
                "",
                "true",
                "true",
                "false",
                "",
                "0",
                "",
                "true",
            ]
            .join(",")
        );
        assert!(matches!(
            Glossary::from_csv(&csv),
            Err(GlossaryCsvError::InvalidEntry {
                row: 2,
                error: GlossaryError::CredentialLikeValue("source term")
            })
        ));
        assert_eq!(
            Glossary::from_csv(&format!(
                "{}\n\"unterminated,target\n",
                super::GLOSSARY_CSV_HEADER
            )),
            Err(GlossaryCsvError::Malformed)
        );
    }

    #[test]
    fn protected_source_chunks_on_semantic_boundaries_without_splitting_markers() {
        let glossary = Glossary::new(vec![
            GlossaryEntry::new("LinguaMesh", "凌瓦网").expect("glossary entry"),
        ])
        .expect("glossary");
        let protected = protect_source_text_with_glossary(
            "First sentence with LinguaMesh. Second sentence keeps https://example.com/path.",
            None,
            "zh-CN",
            Some(&glossary),
        )
        .expect("protected source");
        let chunks = protected.chunks(48).expect("chunks");
        assert!(chunks.len() > 1);
        let mut restored_text = String::new();
        for chunk in chunks {
            let mut chunk_restorer = chunk.restorer();
            restored_text.push_str(&chunk_restorer.push(chunk.text()).expect("chunk output"));
            restored_text.push_str(&chunk_restorer.finish().expect("chunk complete"));
        }
        assert_eq!(
            restored_text,
            "First sentence with 凌瓦网. Second sentence keeps https://example.com/path."
        );
    }

    #[test]
    fn protected_source_keeps_text_that_fits_one_chunk() {
        let protected = protect_source_text("Hello again");
        let chunks = protected
            .chunks(super::DEFAULT_TRANSLATION_CHUNK_BYTES)
            .expect("single chunk");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text(), "Hello again");
    }

    #[test]
    fn protected_source_rejects_a_chunk_limit_that_cuts_a_marker() {
        let protected = protect_source_text("Keep https://example.com/path unchanged.");
        let marker_size = protected.text().find(' ').expect("marker boundary");
        assert_eq!(
            protected.chunks(marker_size),
            Err(ChunkingError::ProtectedSpanTooLarge)
        );
    }
}
