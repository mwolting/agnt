# agnt

Just barely a coding agent.

## Features

- TUI and GUI modes.
- Provider registry with models.dev + API key support for many providers.
- Codex provider.
- Credential management with API key and OAuth PKCE flows.
- Minimal dependencies.

## Installation

You need a recent Rust toolchain (Rust 2024 edition). Install the CLI directly from GitHub:

```bash
cargo install --git https://github.com/mwolting/agnt
```

## Usage

Run the TUI:

```bash
agnt
```

Start the GUI:

```bash
agnt gui
```

List configured providers and their models:

```bash
agnt providers
```

On first run, agnt will prompt you to authenticate for the default provider. Follow the prompts to enter an API key or complete the OAuth flow.

## Development

Useful commands:

```bash
cargo check
cargo check -p agnt-cli
cargo fmt
cargo clippy --all
cargo run
```

## Project Structure

Crates live in `crates/`:

- `agnt-cli`: terminal app entrypoint (TUI + GUI launcher)
- `agnt-core`: agent orchestration and tool wiring
- `agnt-llm`: provider-agnostic LLM interface types
- `agnt-llm-registry`: provider/model registry and auth resolution
- `agnt-llm-openai`: OpenAI-compatible transport
- `agnt-llm-codex`: Codex provider/model presets
- `agnt-auth`: credential storage + OAuth PKCE flows

## License

MIT. See [LICENSE](LICENSE).
