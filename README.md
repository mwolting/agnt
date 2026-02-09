# agnt

agnt is a Rust workspace that ships a terminal-first agent app with optional desktop UI, plus the supporting crates for LLM providers, auth, and registry wiring.

## Features

- Terminal UI (default) and desktop GUI modes.
- Provider registry with OpenAI-compatible and Codex presets.
- Credential management with API key and OAuth PKCE flows.
- Session storage for continuing past conversations.

## Installation

You need a recent Rust toolchain (Rust 2024 edition). Install the CLI directly from GitHub:

```bash
cargo install --git https://github.com/mwolting/agnt
```

## Usage

Run the terminal UI (default):

```bash
agnt
```

Start the desktop GUI:

```bash
agnt gui
```

List known providers and models:

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
