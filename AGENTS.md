# Repository Guidelines

## Project Structure & Module Organization
This is a Rust workspace (`Cargo.toml` at repo root) with crates under `crates/`:
- `agnt-cli`: terminal app entrypoint (`src/main.rs`, TUI in `src/ui.rs`)
- `agnt-core`: agent orchestration and tool wiring
- `agnt-llm`: provider-agnostic LLM interface types
- `agnt-llm-registry`: provider/model registry and auth method resolution
- `agnt-llm-openai`: OpenAI-compatible transport implementation
- `agnt-llm-codex`: Codex-specific registration/auth presets
- `agnt-auth`: credential storage + OAuth PKCE flows

Keep provider-specific behavior in provider crates; keep generic auth/registry logic in `agnt-auth` and `agnt-llm-registry`.

## Build, Test, and Development Commands
- `cargo check`: fast workspace compile check
- `cargo check -p <crate>`: check one crate (example: `cargo check -p agnt-cli`)
- `cargo fmt`: format all Rust code
- `cargo clippy --all`: lint everything
- `cargo run`: run the CLI

Dependency policy: use `cargo add` for new dependencies (do not edit versions manually).

## Coding Style & Naming Conventions
- Rust 2024 edition conventions, 4-space indentation.
- Prefer small, focused modules and explicit types at API boundaries.
- Naming:
  - `snake_case` for functions/modules/files
  - `PascalCase` for structs/enums/traits
  - `SCREAMING_SNAKE_CASE` for constants
- Run `cargo fmt` and `cargo clippy` at natural checkpoints to validate your changes.

## Testing Guidelines
No need for unit tests.

