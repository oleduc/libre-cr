# Configuration

Two TOML files in `~/.config/libre-cr/` configure the system: `review.toml` (review daemon: provider, limits, server) and `code.toml` (code daemon: storage, worktree eviction, repo discovery). Both files are optional — every field has a default — and partial files are fine: set only what you want to change.

## Where everything lives

| Path | Contents |
|---|---|
| `~/.config/libre-cr/review.toml` | Review daemon config (this page) |
| `~/.config/libre-cr/code.toml` | Code daemon config (this page) |
| `~/.config/libre-cr/token` | Daemon bearer token (mode 0600, generated) |
| `~/.config/libre-cr/endpoint` | The daemon's current `http://127.0.0.1:<port>` URL |
| `~/.config/libre-cr/install_key` | Local key used to encrypt your API key at rest |
| `~/.local/share/libre-cr-review/state.db` | SQLite: sessions, turns, notes, tool traces |
| `~/.local/share/libre-cr-code/state.db` | SQLite: known repos, worktrees |
| `~/.local/share/libre-cr-code/repos/` | Auto-cloned repos |
| `~/.local/share/libre-cr-code/worktrees/` | Per-PR worktrees (LRU-evicted) |
| `~/.local/state/libre-cr/log/` | `libre-cr-review.log`, `libre-cr-code.log`, `supervisor.log` |
| `~/.local/state/libre-cr/run/review.pid` | Supervisor PID file |

XDG overrides (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`) are honored; Windows uses the platform equivalents.

## The config UI

```sh
libre-cr config
```

opens `<endpoint>/config-ui?token=…` in your browser — a small form for the provider settings (kind, model, max tokens, temperature, endpoint, API key). Saving writes `review.toml` immediately and the running daemon **hot-swaps the provider without a restart**. An invalid provider kind is rejected with an error before anything is written.

The extension popup's *Configure daemon* link opens the same page.

### Fetch models

Next to the **Model** field is a **Fetch models** button. Click it to ask the live provider API for the models available to your account and fill the dropdown — no more guessing or copy-pasting model ids. It uses the form's current `kind`, `endpoint`, and (if you typed one) `api_key`, so you can list models for a candidate provider *before* saving. Fetching never persists anything.

- **Anthropic** → `GET /v1/models` (returns model ids and human display names).
- **OpenAI-compatible** (including OpenAI, OpenRouter, Ollama) → `GET /v1/models` derived from your endpoint (ids only).

If the fetch fails (bad key, offline, a provider that doesn't expose `/models`), the page shows the error and you can still type the model id by hand — pick **Other / type manually** in the dropdown and use the text field. The free-text field is always the source of truth, so a model that isn't in the list still works.

### Detected credentials and the env-var fallback

You don't have to paste an API key at all if the daemon already has one in its environment. Before starting the daemon, export the standard variable for your provider:

- `anthropic` → `ANTHROPIC_API_KEY`
- `openai_compat` → `OPENAI_API_KEY`

```sh
export ANTHROPIC_API_KEY=sk-ant-…
libre-cr start
```

When you build a provider whose stored key is blank, the daemon falls back to this environment variable automatically. A **stored key always wins** — clearing the env-var fallback is as simple as saving a key through the config UI.

The config UI detects these variables: if the selected provider has one set in the daemon's environment, an inline hint appears next to the API-key field — *"✓ ANTHROPIC_API_KEY detected in the daemon's environment — leave the key blank to use it."* Leave the field blank to use the ambient key.

> Only these two environment variables are inspected. The daemon intentionally does **not** read Claude Code / Claude CLI OAuth credentials, system keychains, or anything under `~/.claude/`.

## `review.toml` reference

### `[provider]`

| Field | Default | Meaning |
|---|---|---|
| `kind` | `"mock"` | `"mock"`, `"anthropic"`, or `"openai_compat"` |
| `model` | `"mock-model"` | Model name sent to the provider (e.g. `claude-sonnet-4-20250514`, `gpt-4o`, `llama3.1`) |
| `max_tokens` | `4096` | Per-response output token cap |
| `temperature` | `0.0` | Sampling temperature |
| `endpoint` | `""` | Override the provider URL (blank = provider default). Either the base URL ending in `/v1` (e.g. `https://openrouter.ai/api/v1`, `http://127.0.0.1:11434/v1`) or the full request URL (`…/v1/chat/completions`, `…/v1/messages`). Required for OpenRouter, Ollama and other non-default endpoints |
| `api_key_enc` | `""` | Your API key, **encrypted at rest** with the install key. Do not hand-edit — set the key through the config UI (or `POST /v1/config` with a plaintext `provider.api_key`, which the daemon encrypts before writing). When left blank, the daemon falls back to the `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` environment variable for the provider kind (see [Detected credentials](#detected-credentials-and-the-env-var-fallback)) |

#### Anthropic

In the config UI: kind `anthropic`, model e.g. `claude-sonnet-4-20250514`, paste your `sk-ant-…` key, leave endpoint blank.

#### OpenAI

Use kind `openai_compat` with endpoint blank-or-`https://api.openai.com/v1`, model e.g. `gpt-4o`, and your OpenAI key. (There is no separate `"openai"` kind — OpenAI is served by the compatible client.)

#### Ollama / other OpenAI-compatible servers

Kind `openai_compat`, endpoint `http://127.0.0.1:11434/v1` (Ollama's OpenAI-compatible API), model e.g. `llama3.1`, any non-empty string as the key if your server ignores auth.

### `[server]`

| Field | Default | Meaning |
|---|---|---|
| `bind` | `"127.0.0.1"` | Bind address. Loopback only by default; change it and you own the network ACLs |
| `port` | `0` | `0` = OS-assigned ephemeral port each start (recorded in the endpoint file). Set a fixed port if you prefer stability |
| `endpoint_file` | `"~/.config/libre-cr/endpoint"` | Where the daemon writes its URL |
| `token_file` | `"~/.config/libre-cr/token"` | Where the bearer token lives |
| `install_key_file` | `"~/.config/libre-cr/install_key"` | Key used to encrypt `api_key_enc` |
| `extension_origin` | `""` | Browser-extension origin allowed by CORS — see below |

#### `extension_origin` and CORS

The daemon only accepts cross-origin requests from one extension origin (e.g. `chrome-extension://abcdef…`). **You normally never set this by hand**: when the extension pairs, it sends its own origin and the daemon applies it to the live CORS layer *and* persists it into `review.toml` automatically, so it survives restarts. Set it manually only if you sideload a rebuilt extension whose ID changed and want to avoid re-pairing — or just re-pair.

### `[storage]`

| Field | Default | Meaning |
|---|---|---|
| `data_dir` | `"~/.local/share/libre-cr-review"` | Review daemon data directory |
| `db` | `"~/.local/share/libre-cr-review/state.db"` | SQLite database path |

### `[code_daemon]`

| Field | Default | Meaning |
|---|---|---|
| `mode` | `"spawn"` | `"spawn"` = review daemon launches `libre-cr-code` itself (normal). External-socket mode is for advanced setups |
| `binary` | `"libre-cr-code"` | Binary name/path to spawn — must resolve on PATH |
| `external_socket` | `""` | Socket path when not spawning |
| `restart_on_failure` | `true` | Restart the code daemon if it dies |
| `max_restarts_per_hour` | `5` | Restart budget before giving up |

### `[mcp_server]`

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Expose the daemon's MCP surface |
| `stdio` | `true` | MCP over stdio |
| `sse` | `true` | MCP over SSE |

### `[global_instructions]`

| Field | Default | Meaning |
|---|---|---|
| `text` | `""` | Free text appended to the assistant's system prompt for **every** question — team conventions, style rules, "we use tabs", etc. |

Example:

```toml
[global_instructions]
text = """
We target Go 1.22. Anything using deprecated ioutil is a finding.
Our test files end in _test.go only; ignore the legacy spec/ tree.
"""
```

### `[limits]`

| Field | Default | Meaning |
|---|---|---|
| `max_tool_turns` | `25` | Max tool-call rounds the agent may take per question |
| `max_history_messages` | `30` | Conversation messages replayed as context per question |
| `session_idle_evict_days` | `90` | Sessions idle longer than this are cleaned up |

## Mock provider mode

The default config (`kind = "mock"`) needs no network and no API key — useful for verifying the whole pipeline (extension → daemon → panel) before spending tokens. The mock provider replays a script you define in `review.toml`; each burst of events up to a `done` answers one question:

```toml
[provider]
kind = "mock"

[[mock.provider_script]]
delay_ms = 200
event = { type = "text_delta", text = "This is a scripted mock answer. " }

[[mock.provider_script]]
event = { type = "text_delta", text = "Your pipeline works end-to-end." }

[[mock.provider_script]]
event = { type = "done" }
```

Event types: `text_delta { text }`, `tool_use { id, name, input }`, `done`, `error { message }`. With an empty script, questions complete immediately with no text. `mock.code_intel = true` additionally fakes worktree readiness (used by tests). Remember to switch `kind` to a real provider afterwards — "I'm still getting mock answers" is the most common gotcha (see [Troubleshooting](troubleshooting.md)).

## `code.toml` reference

### `[storage]`

| Field | Default | Meaning |
|---|---|---|
| `data_dir` | `"~/.local/share/libre-cr-code"` | Repos + worktrees live under here |
| `state_db` | `"~/.local/share/libre-cr-code/state.db"` | SQLite database path |

### `[worktrees]`

| Field | Default | Meaning |
|---|---|---|
| `max_total_bytes` | `5368709120` (5 GiB) | Total worktree disk budget; least-recently-used worktrees are evicted past it |
| `eviction_check_interval_secs` | `3600` | How often the eviction sweep runs |

### `[discovery]`

| Field | Default | Meaning |
|---|---|---|
| `default_roots` | `["~/code", "~/Dev", "~/src"]` | Directories scanned to find your local checkouts. Add the parent dirs of your repos here if discovery misses them |

### `[grep]`

| Field | Default | Meaning |
|---|---|---|
| `default_max_matches` | `200` | Cap on grep results per tool call |

### `[ast_grep]`

| Field | Default | Meaning |
|---|---|---|
| `ast_cache_size` | `256` | Parsed-AST LRU cache entries |

### `[logging]`

| Field | Default | Meaning |
|---|---|---|
| `file` | `false` | Code-daemon file logging toggle (its stderr is already captured into `libre-cr-code.log` by the review daemon) |

## Applying changes

- **Provider settings** saved through the config UI (or `POST /v1/config`): applied live, no restart.
- **Everything else** (`server`, `limits`, `code.toml`, …) edited by hand: run `libre-cr restart`.
