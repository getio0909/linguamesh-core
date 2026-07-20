#![doc = "版本化提供商目录的解析和验证。"]

use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;

const BUNDLED_CATALOG: &str = include_str!("../../../assets/provider-catalog/catalog.json");
const BUNDLED_SCHEMA: &str = include_str!("../../../assets/provider-catalog/schema.json");

/// 当前支持的目录架构版本。
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

/// 表示经过验证的提供商目录。
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalog {
    /// 数据结构版本。
    pub schema_version: u32,
    /// 独立发布的目录语义版本。
    pub catalog_version: String,
    /// 可选择的无秘密提供商预设。
    pub providers: Vec<ProviderPreset>,
}

/// 表示目录中的一个无秘密预设。
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreset {
    /// 稳定的小写预设标识。
    pub id: String,
    /// 面向用户的名称。
    pub display_name: String,
    /// 核心适配器类型。
    pub adapter: String,
    /// 模型发现策略。
    pub model_listing: String,
    /// 可选的提供商文档链接。
    pub documentation_url: Option<String>,
    /// 是否仅允许回环地址。
    #[serde(default)]
    pub loopback_only: bool,
}

/// 描述目录解析或约束失败。
#[derive(Debug, Error, Eq, PartialEq)]
pub enum CatalogError {
    /// JSON 不符合声明的封闭结构。
    #[error("Provider catalog JSON is invalid: {0}")]
    InvalidJson(String),
    /// 架构版本不受支持。
    #[error("Provider catalog schema version {actual} is unsupported; expected {expected}.")]
    UnsupportedSchema {
        /// 当前实现要求的版本。
        expected: u32,
        /// 文件声明的版本。
        actual: u32,
    },
    /// 目录语义版本格式无效。
    #[error("Provider catalog version is invalid.")]
    InvalidCatalogVersion,
    /// 预设标识重复或格式无效。
    #[error("Provider preset ID is invalid or duplicated: {0}")]
    InvalidPresetId(String),
    /// 必填文本字段为空。
    #[error("Provider preset contains an empty required field: {0}")]
    EmptyRequiredField(String),
    /// 文档地址不是受支持的安全地址。
    #[error("Provider documentation URL is invalid: {0}")]
    InvalidDocumentationUrl(String),
}

impl ProviderCatalog {
    /// 解析并验证目录文本。
    pub fn parse(input: &str) -> Result<Self, CatalogError> {
        let catalog: Self = serde_json::from_str(input)
            .map_err(|error| CatalogError::InvalidJson(error.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// 加载编译进核心的目录。
    pub fn bundled() -> Result<Self, CatalogError> {
        Self::parse(BUNDLED_CATALOG)
    }

    /// 返回供工具验证的架构文档。
    #[must_use]
    pub const fn bundled_schema() -> &'static str {
        BUNDLED_SCHEMA
    }

    fn validate(&self) -> Result<(), CatalogError> {
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            return Err(CatalogError::UnsupportedSchema {
                expected: CATALOG_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        if !is_semantic_version(&self.catalog_version) {
            return Err(CatalogError::InvalidCatalogVersion);
        }
        let mut ids = HashSet::new();
        for provider in &self.providers {
            if !is_stable_id(&provider.id) || !ids.insert(provider.id.as_str()) {
                return Err(CatalogError::InvalidPresetId(provider.id.clone()));
            }
            for (name, value) in [
                ("display_name", provider.display_name.as_str()),
                ("adapter", provider.adapter.as_str()),
                ("model_listing", provider.model_listing.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(CatalogError::EmptyRequiredField(name.into()));
                }
            }
            if let Some(url) = &provider.documentation_url
                && (!url.starts_with("https://") || url.chars().any(char::is_whitespace))
            {
                return Err(CatalogError::InvalidDocumentationUrl(url.clone()));
            }
        }
        Ok(())
    }
}

fn is_semantic_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

fn is_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

#[cfg(test)]
mod tests {
    use super::{CatalogError, ProviderCatalog};

    #[test]
    fn bundled_catalog_and_schema_are_valid_json() {
        let catalog = ProviderCatalog::bundled().expect("catalog");
        assert_eq!(catalog.providers.len(), 6);
        let schema: serde_json::Value =
            serde_json::from_str(ProviderCatalog::bundled_schema()).expect("schema");
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    }

    #[test]
    fn credential_shaped_unknown_fields_are_rejected() {
        let input = r#"{
            "schema_version": 1,
            "catalog_version": "1.0.0",
            "providers": [{
                "id": "unsafe-provider",
                "display_name": "Unsafe",
                "adapter": "test",
                "model_listing": "test",
                "api_key": "must-not-be-accepted"
            }]
        }"#;
        assert!(matches!(
            ProviderCatalog::parse(input),
            Err(CatalogError::InvalidJson(_))
        ));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let input = r#"{
            "schema_version": 1,
            "catalog_version": "1.0.0",
            "providers": [
                {"id":"same","display_name":"One","adapter":"a","model_listing":"m"},
                {"id":"same","display_name":"Two","adapter":"a","model_listing":"m"}
            ]
        }"#;
        assert_eq!(
            ProviderCatalog::parse(input),
            Err(CatalogError::InvalidPresetId("same".into()))
        );
    }

    #[test]
    fn insecure_documentation_url_is_rejected() {
        let input = r#"{
            "schema_version": 1,
            "catalog_version": "1.0.0",
            "providers": [{
                "id":"provider",
                "display_name":"Provider",
                "adapter":"adapter",
                "model_listing":"models",
                "documentation_url":"http://example.com/docs"
            }]
        }"#;
        assert_eq!(
            ProviderCatalog::parse(input),
            Err(CatalogError::InvalidDocumentationUrl(
                "http://example.com/docs".into()
            ))
        );
    }
}
