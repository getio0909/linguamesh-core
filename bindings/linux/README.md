# Linux Direct Rust Integration

Linux clients should call the typed Rust application layer directly. Do not route normal GTK work
through the C ABI and do not reimplement provider behavior in the client. During workspace
development, the sibling `linguamesh-linux` manifest can use pinned path dependencies:

```toml
[dependencies]
linguamesh-application = { version = "=0.1.0-alpha.2", path = "../linguamesh-core/crates/linguamesh-application" }
linguamesh-domain = { version = "=0.1.0-alpha.2", path = "../linguamesh-core/crates/linguamesh-domain" }
linguamesh-engine = { version = "=0.1.0-alpha.2", path = "../linguamesh-core/crates/linguamesh-engine" }
linguamesh-storage = { version = "=0.1.0-alpha.2", path = "../linguamesh-core/crates/linguamesh-storage" }
tokio-util = "0.7"
```

Connect through the application manager, including for credential-free loopback providers:

```rust
use linguamesh_application::{ProviderManager, host_secret_channel};
use linguamesh_domain::{ProviderProfile, ProviderProfileId, TranslationRequest};
use tokio_util::sync::CancellationToken;

let (broker, _host_requests) = host_secret_channel(8)?;
let mut providers = ProviderManager::new(broker);
let profile = ProviderProfile::new(
    ProviderProfileId::parse("local-loopback")?,
    "Local provider",
    "local-loopback",
    "openai_chat_completions",
    endpoint,
    None,
)?;
let cancellation = CancellationToken::new();
let models = providers.connect(&profile, &cancellation).await?;
let selected_model_id = selected_model_id_from_ui;
let selected_model = models
    .iter()
    .find(|model| model.id == selected_model_id)
    .ok_or("The selected model is unavailable.")?;
let mut operation = providers
    .active_engine()
    .expect("connected provider")
    .translate(TranslationRequest::new(text, "zh-CN", selected_model.id.clone()));
while let Some(event) = operation.next_event().await {
    // 将事件送入后台合并和节流队列。
    sender.send(event).await?;
}
providers.disconnect();
```

Keep the returned `HostSecretRequests` on a bounded worker even when the initial profile needs no
secret; the underscore in the abbreviated example is not production lifecycle guidance. For a
credential-bearing profile, create the reference with
`SecretRef::new(SecretRefNamespace::SecretService)`, store the value in Secret Service, and answer
only the matching lease with a one-time `SecretValue`. Keep network and event work off the GTK main
context. Require a deliberate model selection, persist its ID per provider profile, and validate
that exact ID against discovery or the supported manual fallback before each request; never select
the first discovered model implicitly. The downstream queue, not `sender.send`, performs UI event
batching and throttling. Never put credential values in the core database. Release builds must pin
an immutable Core version and source revision from the central compatibility record rather than a
moving branch.

`bash tools/package-linux-sdk.sh` additionally creates a C/static/shared-library bundle for
non-Rust consumers. That bundle is not needed by the normal Rust/GTK client.
