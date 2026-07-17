#![doc = "`LinguaMesh` 的稳定领域类型。"]

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

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
    use super::{ErrorKind, TranslationError, TranslationEvent};

    #[test]
    fn terminal_events_are_classified() {
        let failed = TranslationEvent::Failed {
            sequence: 4,
            error: TranslationError::new(ErrorKind::Network, "Network failed."),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.sequence(), 4);
    }
}
