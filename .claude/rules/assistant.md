---
paths:
  - "crates/caditor-ai/**"
  - "crates/caditor/src/assistant.rs"
  - "crates/caditor/src/assistant/**"
  - "crates/caditor/src/assistant_panel.rs"
  - "crates/caditor/src/mcp/bridge.rs"
  - "crates/caditor/src/preferences/assistant.rs"
  - "crates/caditor/src/ui_tests/assistant.rs"
---

# In-editor AI assistant and live MCP bridge

## `caditor-ai`

Pure-Rust streaming LLM client crate with no UI or CAD dependency, built on the workspace's `ureq`,
`rustls-graviola` and `webpki-roots` stack.

- `ApiProvider` covers Anthropic Claude (`anthropic.rs`, Messages API SSE), Google Gemini
  (`gemini.rs`, Generative Language SSE with `normalize_schema` converting union type arrays into
  `anyOf` and stripping `additionalProperties`), OpenAI GPT (`openai.rs`, Chat Completions SSE) and
  `ApiProvider::Custom` (OpenAI-compatible endpoint).
- `Network` (`network.rs`) enforces HTTPS for all remote hosts and allows `http://` only for
  loopback hosts (`localhost`, `127.0.0.1`, `[::1]`) so local servers such as Ollama and LM Studio
  work without weakening remote TLS.
- `ApiKey` and `HttpRequest` redact credentials and bodies in `Debug`. Every failure is a typed
  `AiError`. `fake.rs` (`test-support` feature) supplies `ScriptedTransport` for deterministic tests.

## Live MCP execution and loopback bridge

- `mcp/session.rs` supports `Session::attached`, an in-memory mode over a snapshot of the live open
  `Document` and `Evaluation` that records every `Change` (`Apply`, `Undo`, `Redo`) and delegates
  `save` / `save_to` through `SaveHook` instead of spawning a second recovery journal.
- `assistant/executor.rs` (`LiveMcpExecutor`) runs all 62 MCP tools plus `list_resources` and
  `read_resource` on background worker threads, requesting a settled `ModelSnapshot` from the UI
  thread, committing `Change`s through `Action::Apply` / `Undo` / `Redo` (landing in real Undo
  history and the recovery journal), and decoding `render_view` PNGs into `Thumbnail`s off the UI
  thread.
- `mcp/bridge.rs` (`LoopbackBridge`) binds `127.0.0.1:0` with a 32-byte random hex token generated
  by `graviola::random::fill`. Local CLIs (`claude`, `gemini`, `codex`) connect via
  `caditor mcp --connect <port>` with the token passed in `CADITOR_MCP_TOKEN` (never argv),
  speaking JSON-RPC 2.0 against the same `LiveMcpExecutor`.

## Key storage, CLI runner and chat UI

- `assistant/keys.rs` (`KeyStore`) stores API keys in the operating system's keyring via
  `cloud::keyring::AiKeyring` (freedesktop Secret Service on Linux, Credential Manager on Windows)
  with an owner-only private file fallback in `<state>/assistant/` and optional environment variable
  fallback (`ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `OPENAI_API_KEY`). Secrets never enter
  `preferences.json`.
- `assistant/cli_runner.rs` discovers and spawns `claude`, `gemini` and `codex` without a shell,
  streaming output live and terminating the child process on Cancel. Tests inject a fake
  `CliBackend`.
- `assistant_panel.rs` draws the dockable right-hand chat side panel strictly through `widgets.rs`
  and `appearance::tokens`, toggled by `Command::Assistant` and cancellable by
  `Command::CancelAssistant`. Messages are `widgets::card`s, tool activity and code
  `widgets::well`s; the provider and model combos are named (`PROVIDER`, `MODEL`); the busy
  transcript shows progress without its own Cancel, the composer's Stop being the one control;
  Enter in the composer is always taken (sending only when it can) and the hint wraps beside Send
  at any size; disabled controls say why on hover. It is a `guide::SidePanel`, so F1 opens the
  Assistants page while it is open.
- `preferences/assistant.rs` holds non-secret assistant settings under `Section::Assistant` in
  Preferences › General.
