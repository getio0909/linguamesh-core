#![doc = "`LinguaMesh` 的稳定领域类型。"]

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use url::{Host, Url};
use uuid::{Uuid, Variant, Version};

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

/// 描述不含任何凭据值的规范提供商配置。
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderProfile {
    id: ProviderProfileId,
    display_name: String,
    preset_id: String,
    adapter_type: String,
    base_endpoint: EndpointConfiguration,
    secret_ref: Option<SecretRef>,
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
}

/// 验证即将写入配置存储的模型标识不包含凭据形态。
pub fn validate_model_identifier(value: &str) -> Result<(), ProfileValidationError> {
    checked_profile_text(value.to_owned(), "model_id").map(drop)
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

/// 表示一个必须在翻译输出中原样恢复的非语言片段。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedSpan {
    /// 发送给模型的不可变占位符。
    token: String,
    /// 用户源文本中的原始片段。
    source: String,
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
}

/// 扫描常见结构化片段并替换为不透明占位符。
#[must_use]
pub fn protect_source_text(source: &str) -> ProtectedSource {
    let mut candidates = Vec::new();
    collect_fenced_code_candidates(source, &mut candidates);
    collect_inline_code_candidates(source, &mut candidates);
    collect_placeholder_candidates(source, &mut candidates);
    collect_url_candidates(source, &mut candidates);
    collect_email_candidates(source, &mut candidates);

    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    let mut selected = Vec::new();
    for (start, end) in candidates {
        if start < end
            && selected
                .last()
                .is_none_or(|(_, previous_end)| start >= *previous_end)
        {
            selected.push((start, end));
        }
    }

    let mut protected_text = String::with_capacity(source.len());
    let mut spans = Vec::with_capacity(selected.len());
    let mut cursor = 0;
    for (index, (start, end)) in selected.into_iter().enumerate() {
        protected_text.push_str(&source[cursor..start]);
        let mut token = format!("{PROTECTED_TOKEN_PREFIX}{index}{PROTECTED_TOKEN_SUFFIX}");
        while source.contains(&token) {
            token.push('_');
        }
        protected_text.push_str(&token);
        spans.push(ProtectedSpan {
            token,
            source: source[start..end].to_owned(),
        });
        cursor = end;
    }
    protected_text.push_str(&source[cursor..]);
    ProtectedSource {
        text: protected_text,
        spans,
    }
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
            output.push_str(&self.spans[index].source);
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
        }
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
}

impl TranslationError {
    /// 创建已归一化错误。
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
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
            .finish()
    }
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
            | Self::Completed { sequence }
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
        CompatibilityError, CompatibilityRequirements, CoreCompatibility, ErrorKind,
        ProfileValidationError, ProtectedTextError, ProviderProfile, ProviderProfileId, SecretRef,
        SecretRefNamespace, SecretValue, TranslationError, TranslationEvent, protect_source_text,
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
}
