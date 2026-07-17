# Linux Direct Rust Integration

Linux clients should call the typed Rust application layer directly. Do not route normal GTK work
through the C ABI and do not reimplement provider behavior in the client. During workspace
development, the sibling `linguamesh-linux` manifest can use pinned path dependencies:

```toml
[dependencies]
linguamesh-domain = { version = "=0.1.0-alpha.1", path = "../linguamesh-core/crates/linguamesh-domain" }
linguamesh-engine = { version = "=0.1.0-alpha.1", path = "../linguamesh-core/crates/linguamesh-engine" }
linguamesh-provider-openai = { version = "=0.1.0-alpha.1", path = "../linguamesh-core/crates/linguamesh-provider-openai" }
```

Construct the same public engine used behind the native ABI:

```rust
use linguamesh_domain::TranslationRequest;
use linguamesh_engine::TranslationEngine;
use linguamesh_provider_openai::{OpenAiCompatibleProvider, OpenAiConfig};
use std::sync::Arc;

let provider = OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(endpoint))?;
let engine = TranslationEngine::new(Arc::new(provider));
let mut operation = engine.translate(TranslationRequest::new(text, "zh-CN", model));
while let Some(event) = operation.next_event().await {
    // 将事件批量转发到 GLib 主上下文。
    sender.send(event).await?;
}
```

Keep network and event work off the GTK main context. Store credentials in Secret Service and pass
them through an in-memory broker; never put values in the core database. Release builds must pin an
immutable core version and source revision from the central compatibility record rather than a
moving branch.

`bash tools/package-linux-sdk.sh` additionally creates a C/static/shared-library bundle for
non-Rust consumers. That bundle is not needed by the normal Rust/GTK client.
