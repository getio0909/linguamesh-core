#![doc = "提供商协议适配器的稳定抽象。"]

use async_trait::async_trait;
use futures_core::Stream;
use linguamesh_domain::{ModelDescriptor, TranslationError, TranslationRequest};
use std::pin::Pin;
use tokio_util::sync::CancellationToken;

/// 包装增量翻译文本流。
pub type TranslationStream =
    Pin<Box<dyn Stream<Item = Result<String, TranslationError>> + Send + 'static>>;

/// 定义引擎需要的提供商能力。
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// 列出当前可用模型。
    async fn list_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError>;

    /// 提交请求并返回真实增量流。
    async fn translate_stream(
        &self,
        request: TranslationRequest,
        cancellation: CancellationToken,
    ) -> Result<TranslationStream, TranslationError>;
}
