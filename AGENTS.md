# LinguaMesh Core Instructions

Read `../linguamesh-project/PROJECT_GOAL.md` and this repository's `GLOBAL_GOAL.md`, `REPOSITORY_ROLE.md`, and `IMPLEMENTATION_STATUS.md` before changes. Preserve unrelated work and record assumptions with `Assumption:`.

Keep provider protocols, routing, translation, persistence, document processing, typed errors, and native-boundary behavior in Rust. Public APIs require documentation. All code comments must be Simplified Chinese on separate lines above the code they describe. Console, log, diagnostic, and CLI output must be English.

Run before committing:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --workspace --locked
```

Never log credentials or source content. Unsafe Rust is limited to reviewed FFI/platform boundaries; every unsafe block needs a meaningful Simplified-Chinese `SAFETY` comment. A panic must never cross FFI. Do not commit or push a checkpoint until tests, documentation, and implementation evidence agree.
