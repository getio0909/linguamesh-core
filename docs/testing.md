# Testing

Default tests use only local deterministic fixtures and the loopback fake provider. They must not require commercial credentials or the public internet. Provider contract tests cover fragmented SSE, split UTF-8, malformed messages, disconnects, errors, response limits, and cancellation. Persistence tests use isolated temporary or in-memory SQLite databases.
