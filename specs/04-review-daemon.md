# Review Daemon (`libre-cr-review`)

## Purpose

The product-specific brain. The review daemon hosts the LLM client, runs the agent loop, holds per-PR conversation state, and orchestrates the code daemon as an MCP client. It serves the browser extension over HTTP/WebSocket and exposes a smaller MCP surface for external AI clients.

If `libre-cr-code` is "an MCP server for any repo," `libre-cr-review` is "the assistant that knows about PRs and conversations."

## Process Model

- A single Rust binary: `libre-cr-review`.
- One mode: long-running server. Started on demand or as a login item.
- On startup:
  1. Read config; ensure data dir exists.
  2. Generate bearer token if missing; write to `~/.config/libre-cr/token` (mode 0600).
  3. Open SQLite DB; run migrations.
  4. Spawn child `libre-cr-code` as MCP-over-stdio (or connect to configured external instance).
  5. Bind HTTP listener on configured `127.0.0.1:<port>`. Default: ephemeral port; resolved port written to `~/.config/libre-cr/endpoint` for the extension to read.
  6. If MCP server is enabled, set up stdio/SSE MCP handlers.
- On shutdown: drain in-flight requests, gracefully close MCP child, flush SQLite.

## HTTP / WebSocket API (Extension Transport)

All endpoints except `POST /v1/pair` (code redemption — how the extension obtains a token; rate-limited) and `GET /v1/health` require `Authorization: Bearer <token>`. CORS is permissive (`*`): the bearer token is the security boundary, and a content script's requests carry the page origin (`https://github.com`), not the extension's, so an origin allowlist cannot work. All responses are JSON unless noted.

### Sessions

A **session** corresponds 1:1 with a PR (identified by `pr_url`). It holds the conversation history and a pointer to the worktree the code daemon prepared for it.

- **`POST /v1/sessions`**
  Body: `{ pr_url: string, pr_data: ScrapedPRData }`
  Creates or updates a session for this PR. The extension calls this on PR page load with everything it could scrape. Response includes the session id and worktree readiness state.
  Returns: `{ session_id, worktree_ready: bool, repo_local_path: string | null, pending_action: string | null, pr_diff_changed: bool, head_sha: string | null }`. `pending_action` is `"worktree_pending"` while the worktree is being prepared in the background; `pr_diff_changed` is `true` when the incoming `head_sha` differs from the one previously recorded for this PR (drives the "PR diff changed" banner).

- **`GET /v1/sessions/:id`**
  Full session state: PR metadata, conversation turns, current worktree path.

- **`GET /v1/sessions`**
  List sessions (recent first). Supports `?limit=`, `?since=`.

- **`DELETE /v1/sessions/:id`**
  Drop conversation and any user notes for this PR. Does not remove the worktree (that's the code daemon's eviction job).

### Ask / streaming Q&A

- **`WS /v1/sessions/:id/ask`**
  WebSocket upgrade. The client opens it once per Q&A turn (not per session — a fresh socket per question keeps state simple and lets either side disconnect cleanly).

  Client → server (first frame): `{ question: string, selection?: Selection, verb?: string, mute_presentations?: bool }`

  When `mute_presentations` is `true` the daemon does not register the presentation tools for that turn at all, so the model cannot emit `presentation_call` frames — the mute toggle genuinely suppresses presentations rather than relying on the extension to ignore them. The extension also gates locally as defense in depth: while muted it answers any stray `presentation_call` with `{ ok: false, error: "presentation_muted" }`.

  Server → client frames:
  ```
  { type: "tool_call",          name, input, call_id }
  { type: "tool_result",        call_id, result_preview }
  { type: "text_delta",         text }
  { type: "presentation_call",  call_id, tool, input }     ← needs client reply
  { type: "done",               turn_id, usage: { input_tokens, output_tokens } }
  { type: "error",              message, recoverable: bool }
  ```

  Additional client → server frames (after the initial `{question,…}`):
  ```
  { type: "presentation_result", call_id, ok, result?, error?, message? }
  ```

  `presentation_call` frames carry tool calls the LLM made that target the
  extension (highlight, annotate, scroll, open_link, clear). The extension
  executes them locally and replies with `presentation_result`. See
  `09-presentation-tools.md`.

  The `tool_call` / `tool_result` frames are visible to the UI so the panel can show a "thinking trace" the user can expand. This is intentional — reviewers want to know what the agent looked at before trusting the answer.

  Cancellation: the client closes the socket. The server aborts the in-flight LLM call and any pending tool dispatches, but persists the partial turn (with status `cancelled`) so it remains in conversation history.

### Notes

- **`POST /v1/sessions/:id/notes`**
  Body: `{ content: string, anchor?: Selection }`
  Adds a user note to the conversation timeline. Equivalent to a "turn" without an LLM call. Used by the "I want to remember this" button.

- **`PATCH /v1/sessions/:id/notes/:note_id`**
- **`DELETE /v1/sessions/:id/notes/:note_id`**

### Export

- **`POST /v1/sessions/:id/export`**
  Body: `{ format: "markdown" | "github_review", filter?: { include_thinking: bool, severity_min?: string } }`
  Returns the assembled review draft. Markdown is for clipboard paste; `github_review` is structured for the (future) direct-post path.
  Returns: `{ content, structure?: GithubReviewStructure }`

### Config

- **`GET /v1/config`** — non-sensitive config (provider kind, model, max_tokens, temperature, endpoint, limits, global instructions, mcp_server flags). API key is never returned.
- **`POST /v1/config`** — update provider config from the daemon's own config web UI (NOT the extension; see "Configuration UI" below). The body is a partial `{ provider: { kind?, model?, max_tokens?, temperature?, endpoint?, api_key? } }` patch; a plaintext `api_key` is encrypted before it lands in the config. The route is transactional: it builds a candidate config, *proves a provider constructs from it* (so a typo'd `kind` is rejected with `400` before anything changes), atomically persists `review.toml`, then commits and hot-swaps the running provider. A failed disk write is a `500` with nothing applied — never a silent `{ok:true}` that evaporates on restart. **Provider changes take effect immediately, with no daemon restart** (the running provider is held behind a swappable cell that this route updates).
- **`POST /v1/config/validate`** — validate a provider. With no body it validates the *currently-stored* provider; with a provider patch body it validates that candidate. Either way it builds the provider fresh and calls `validate()` — never the stale startup snapshot.
- **`POST /v1/provider/models`** — list the models offered by a *candidate* provider. The body is a provider patch (same shape as `POST /v1/config`); the daemon applies it to a clone of the current config, builds an ephemeral provider, and returns `{ models: [{ id, display_name? }] }`. Nothing is persisted. Lets the config UI populate a model dropdown for a new provider+key before saving. Providers opt in; ones that don't support listing return a validation error.
- **`GET /v1/provider/detected`** → `{ anthropic, openai }`. Reports which ambient env vars (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`) are set in the daemon's environment so the config UI can offer a "leave the key blank" path.

### Health

- **`GET /v1/health`** → `{ ok, version, protocol_version, code_daemon: { connected, version } }`. `protocol_version` is `libre_cr_common::PROTOCOL_VERSION`; the extension does a soft compatibility check against its own copy. The `code_daemon` block reflects the real child state via the wrapper's health hook (the `{connected:true, version:"mock"}` fallback exists only for in-process tests).
- **`GET /v1/health/code-daemon`** → detailed code-daemon health (`connected`, `version`, `last_error`, `restart_count`).

### Pairing

- **`POST /v1/pair/issue`** — token-authenticated issuer for one-time pairing codes. Body: `{ ttl_seconds?: number }` (clamped to 30 s … 15 min, default 300 s). Returns `{ code, expires_at_epoch_ms }`. The wrapper's `libre-cr pair` (and `libre-cr-review pair`) call this so codes land in the *running* daemon's pairing store — issuing a code locally in a CLI is meaningless, since the extension redeems against the daemon. The requested TTL is actually applied to the stored code, not merely echoed.
- **`POST /v1/pair`** — redeem a pairing code (no bearer token; this is how the extension obtains one). Body: `{ code, extension_origin? }`. Per-code TTL is enforced and failed attempts are rate-limited per source IP (a `429` with `Retry-After` after too many failures in a window). On success returns `{ token, extension_origin }`; a supplied `extension_origin` is persisted to `review.toml` and applied to the live CORS allowlist immediately (no restart).

### Search and verbs

- **`GET /v1/search?q=&limit=`** — cross-session full-text search over the conversation history (SQLite FTS5). Returns `{ results: [{ session_id, pr_url, turn_id, snippet, score }] }`.
- **`GET /v1/verbs`** — the investigation-verb catalog as `[{ id, label, required_selection, description, suggested_tools }]`. See `06-investigation-verbs.md`.

### Configuration UI

- **`GET /config-ui`** — a self-contained static HTML configuration page (no templating). It reads its bearer token from `?token=` and posts to `/v1/config`, fetches `/v1/provider/models` and `/v1/provider/detected`. See § Configuration UI below.

## MCP Server Surface (External Clients)

Exposed on stdio (`libre-cr-review mcp-stdio`) or SSE (`/mcp` on the same HTTP listener, token-auth required for SSE).

Tools:

- **`ask_about_pr`** `{ pr_url, question, selection? }` → `{ answer, turn_id }`
  Runs the same agent loop the extension uses. Creates a session if none exists.
- **`list_sessions`** `{ limit?: number }` → `{ sessions: [...] }`
- **`get_session_history`** `{ pr_url }` → `{ turns: [...] }`
- **`export_session`** `{ pr_url, format }` → `{ content }`

We deliberately do **not** expose the lower-level tools (`grep`, `find_references`, etc.) at this layer. External clients that want raw tools should connect to the code daemon directly. The review daemon's MCP value is "ask the assistant," not "use my tools."

## Internal Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                          libre-cr-review                            │
│                                                                     │
│  ┌────────────────────────┐  ┌──────────────────────────┐           │
│  │  HTTP/WS server        │  │  MCP server (stdio/SSE)  │           │
│  │  (axum)                │  │                          │           │
│  │  • Auth middleware     │  │  • ask_about_pr, …       │           │
│  │  • CORS                │  │                          │           │
│  └────────┬───────────────┘  └─────────────┬────────────┘           │
│           │                                │                        │
│           └────────────┬───────────────────┘                        │
│                        │                                            │
│                        ▼                                            │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Session manager                                              │   │
│  │  • Get/create session for pr_url                              │   │
│  │  • Load history, append turns, update notes                   │   │
│  │  • Coordinate worktree readiness with code daemon             │   │
│  └────────────────┬─────────────────────────────────────────────┘   │
│                   │                                                  │
│                   ▼                                                  │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Agent loop                                                   │   │
│  │  • Build messages from history + selection + verb prompt     │   │
│  │  • Compose tool set: code-daemon tools + internal PR tools   │   │
│  │  • Stream provider response, dispatch tool calls, loop       │   │
│  │  • Emit frames to caller (WS or MCP)                         │   │
│  │  • Persist turn at end                                       │   │
│  └────┬──────────────────────┬─────────────────────┬────────────┘   │
│       │                      │                     │                │
│       ▼                      ▼                     ▼                │
│  ┌──────────┐         ┌─────────────┐       ┌───────────────────┐   │
│  │ Provider │         │ Tool router │       │ Verb registry     │   │
│  │ client   │         │             │       │ • prompt template │   │
│  │ (Anthr,  │         │ • code-d    │       │ • required tools  │   │
│  │  OpenAI) │         │ • internal  │       │ • output schema   │   │
│  └──────────┘         └──────┬──────┘       └───────────────────┘   │
│                              │                                       │
│                  ┌───────────┴────────────┐                          │
│                  ▼                        ▼                          │
│          ┌───────────────┐         ┌──────────────────┐              │
│          │ MCP client    │         │ Internal tools   │              │
│          │ (to code      │         │ • get_pr_diff    │              │
│          │  daemon)      │         │ • get_pr_comments│              │
│          │               │         │ • get_selection  │              │
│          └───────────────┘         │ • add_note       │              │
│                                    └──────────────────┘              │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  SQLite store                                                 │   │
│  │  sessions, turns, notes, tool_traces, providers              │   │
│  └──────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────┘
```

## Agent Loop

Pseudo-Rust for the core loop. The real implementation is ~300 lines including streaming + cancellation + error handling.

```rust
async fn run_turn(
    ctx: &TurnContext,
    user_question: &str,
    selection: Option<Selection>,
    verb: Option<&str>,
    sink: &mut FrameSink,         // sends WS / MCP frames
) -> Result<Turn> {
    let system = build_system_prompt(ctx, verb);
    let history = ctx.session.recent_messages(MAX_HISTORY);
    let user_msg = build_user_message(user_question, selection.as_ref());

    let mut messages = vec![system, ...history, user_msg];
    let tools = ctx.tool_router.tools_for_verb(verb);

    let mut tool_traces = Vec::new();

    for _turn in 0..MAX_TOOL_TURNS {
        let mut text_buf = String::new();
        let mut tool_uses = Vec::new();

        let stream = ctx.provider.stream(&messages, &tools).await?;
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(t) => {
                    sink.text_delta(&t).await?;
                    text_buf.push_str(&t);
                }
                StreamEvent::ToolUse(use_) => tool_uses.push(use_),
                StreamEvent::Done { usage, stop_reason } => {
                    if tool_uses.is_empty() {
                        // Final answer.
                        sink.done(usage).await?;
                        return persist_turn(ctx, text_buf, tool_traces, usage).await;
                    }
                    break;
                }
                StreamEvent::Error(e) => {
                    sink.error(&e, /*recoverable*/ false).await?;
                    return Err(e.into());
                }
            }
        }

        // Dispatch every tool call in this turn concurrently, preserving the
        // model's order for the result blocks (join_all returns outputs in
        // input order).
        let assistant_msg = make_assistant_message(text_buf, &tool_uses);
        messages.push(assistant_msg);

        for tu in &tool_uses { sink.tool_call(tu).await?; }
        let outcomes = join_all(tool_uses.iter().map(|tu| ctx.tool_router.dispatch(tu))).await;

        let mut tool_result_blocks = Vec::new();
        for (tu, result) in tool_uses.iter().zip(outcomes) {
            sink.tool_result(&tu.id, &result).await?;
            tool_traces.push(ToolTrace { call: tu.clone(), result: result.clone() });
            tool_result_blocks.push(make_tool_result_block(&tu.id, &result));
        }
        messages.push(make_user_message_with_tool_results(tool_result_blocks));
    }

    Err(Error::TooManyToolTurns)
}
```

Key properties:

- **Streaming text** flows to the UI as it arrives. Tool calls also stream so the panel can show what's happening.
- **Tool calls within a single LLM turn run in parallel** via `join_all`. A typical "find callers and history" question results in 2-3 concurrent tool dispatches; their result blocks are reassembled in the model's original order.
- **Bounded** at `MAX_TOOL_TURNS = 25`. Past this, the agent has clearly gone off the rails — return an error. (Verbs that legitimately need many turns should chain explicitly, not blow this budget.)
- **Cancellation** is via `tokio::select!` on the client-disconnect signal; partial state is persisted with `cancelled` status.
- **Token usage is real.** The Anthropic stream parser reads `input_tokens` from `message_start` (`message.usage`) and `output_tokens` + `stop_reason` from `message_delta` (the API does not put them on `message_stop`); the OpenAI-compatible parser requests `stream_options: { include_usage: true }`. The final `done` frame and the per-turn `usage_in`/`usage_out` columns carry the actual tallies.
- **Turn ordinals are assigned inside the insert transaction** (`insert_turn_auto_ordinal`) so concurrent note/turn writes on a session can't collide on the `UNIQUE(session_id, ordinal)` constraint.

## Tool Router

The router presents a unified tool surface to the agent, drawing from three categories:

- **Code daemon tools** — discovered at startup by calling `tools/list` over MCP. The router caches the list and the schemas. If the code daemon restarts, the router re-discovers.
- **Internal review-daemon tools** — Rust functions registered at startup. These are PR-aware and session-aware: `get_pr_diff(session_id)`, `get_pr_comments(session_id)`, `get_selection(session_id)`, `add_note(session_id, content)`.
- **Presentation tools** — bounded set that affect what the user sees in the browser. Registered statically in the daemon. Routed *back* over the active WebSocket to the extension; see `09-presentation-tools.md`.

When the agent invokes a tool, the router dispatches by category:

- Code-daemon tools → MCP `tools/call` over stdio to the child process.
- Internal tools → direct Rust call.
- Presentation tools → send a `presentation_call` frame on the current turn's WebSocket, await a `presentation_result` reply (5 s timeout).

The tool's `repo_path` argument is set automatically for tools that take one — the router knows the current session's worktree path and injects it. The agent never has to know the local filesystem layout.

The presentation-tool category is only registered for turns that have an active WebSocket (i.e., reached via the extension's `/v1/sessions/:id/ask`). When the same agent loop is reached via the daemon's external MCP server (`ask_about_pr`), there is no extension to talk to and presentation tools are not registered. The agent simply doesn't see them in that mode.

## Internal Tools (Detailed)

- **`get_pr_diff`** `{}` → `{ files: [{ path, status, additions, deletions, hunks }] }`
  Returns the diff as scraped from the browser (cached in the session). The agent uses this when the user's question doesn't reference a specific selection but is about "this PR."

- **`get_pr_comments`** `{}` → `{ comments: [{ author, body, file?, line?, replies }] }`
  PR conversation comments. Useful for "has this been discussed before?"

- **`get_pr_metadata`** `{}` → `{ title, description, author, base_branch, head_branch, files_changed }`

- **`get_selection`** `{}` → `{ file, start_line, end_line, content } | null`
  The user's selection at question time. Set by the WS handshake.

- **`add_note`** `{ content: string, severity?: string }` → `{ note_id }`
  Lets the agent itself jot down something to surface in the export. The user can also do this manually via the UI. Severity is one of `info`, `suggestion`, `warning`, `critical` — drives sorting and styling in the export draft.

- **`session_history_search`** `{ query: string }` → `{ matches: [{ turn_id, snippet }] }`
  Lets the agent reference past turns ("you said earlier that…"). Hits the SQLite FTS index.

## LLM Provider Layer

Mirrors the POC's shape: a trait with implementations per provider.

```rust
trait Provider: Send + Sync {
    fn id(&self) -> &str;
    async fn stream(&self, messages: &[Message], tools: &[Tool])
        -> Result<impl Stream<Item = Result<StreamEvent>> + Send>;
    async fn validate(&self) -> Result<()>;
    // Providers opt in; the default returns "not supported".
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}
```

Provider kinds for v2 (`provider.kind` in config):

- **`mock`** — no network; canned responses for development and tests.
- **`anthropic`** — official Messages API. Streaming SSE. Tool-use loop. 120 s timeout. Implements `list_models` via `GET /v1/models`.
- **`openai_compat`** — chat completions with tool calls. Works against api.openai.com, OpenRouter, Ollama, and any compatible endpoint.

Both streaming parsers are symmetric on failure: an `event: error` frame becomes a `StreamEvent::Error` carrying the API's message (rather than a generic "ended without done"), and buffered tool-call state is flushed into a `ToolUse` on early EOF instead of being silently dropped. Streamed tool inputs are accumulated per block/call id and emitted as a single well-formed `ToolUse`, never per-fragment.

Credential resolution (`anthropic` / `openai_compat`): the stored (decrypted) key always wins; when it is empty the daemon falls back to the standard ambient environment variable for the kind — `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`. This lets a user with the env var set never paste a key. Env vars only — the daemon never reads Claude Code / Claude CLI credentials, keychains, or `~/.claude/*` (Anthropic's terms restrict subscription OAuth tokens to Claude Code itself).

Configuration includes:
- Model
- Endpoint override
- Max tokens
- Temperature (defaults to 0 for determinism in tool routing; can be raised for free-form explanation verbs)
- Optional system prompt prepended to every turn ("global instructions")

## Conversation Storage (SQLite)

```sql
CREATE TABLE sessions (
  session_id   TEXT PRIMARY KEY,
  pr_url       TEXT NOT NULL UNIQUE,
  pr_owner     TEXT NOT NULL,
  pr_repo      TEXT NOT NULL,
  pr_number    INTEGER NOT NULL,
  repo_id      TEXT,                       -- resolved by code daemon (nullable until ready)
  worktree_path TEXT,                       -- nullable until ready
  pr_data      TEXT NOT NULL,               -- JSON: title, description, author, branches, files
  created_at   INTEGER NOT NULL,
  last_active_at INTEGER NOT NULL
);

CREATE TABLE turns (
  turn_id      TEXT PRIMARY KEY,
  session_id   TEXT NOT NULL REFERENCES sessions(session_id),
  ordinal      INTEGER NOT NULL,            -- monotonically increasing per session
  kind         TEXT NOT NULL,               -- "question" | "note"
  status       TEXT NOT NULL,               -- "ok" | "cancelled" | "error"
  verb         TEXT,                        -- nullable
  question     TEXT,                        -- nullable for notes
  selection    TEXT,                        -- JSON; nullable
  answer       TEXT,                        -- assistant final text; nullable for notes/cancelled
  user_content TEXT,                        -- for kind="note": the user's note body
  severity     TEXT,                        -- for notes / agent-flagged issues
  usage_in     INTEGER,
  usage_out    INTEGER,
  created_at   INTEGER NOT NULL,
  UNIQUE (session_id, ordinal)
);

CREATE TABLE tool_traces (
  trace_id     TEXT PRIMARY KEY,
  turn_id      TEXT NOT NULL REFERENCES turns(turn_id),
  ordinal      INTEGER NOT NULL,
  tool_name    TEXT NOT NULL,
  input_json   TEXT NOT NULL,
  output_json  TEXT NOT NULL,
  duration_ms  INTEGER NOT NULL,
  ok           INTEGER NOT NULL,            -- 0/1
  UNIQUE (turn_id, ordinal)
);

CREATE VIRTUAL TABLE turns_fts USING fts5(
  question, answer, user_content,
  content='turns', content_rowid='rowid'
);
```

Migrations live in a `_schema_version` table; the daemon refuses to run against a newer schema than it knows.

## Configuration

Config file: `~/.config/libre-cr/review.toml`.

```toml
[server]
bind = "127.0.0.1"
port = 0                       # 0 = ephemeral; resolved port written to endpoint file
endpoint_file = "~/.config/libre-cr/endpoint"
token_file = "~/.config/libre-cr/token"
extension_origin = ""          # written by extension on first-run pairing; informational only

[storage]
data_dir = "~/.local/share/libre-cr-review"
db = "~/.local/share/libre-cr-review/state.db"

[provider]
kind = "anthropic"             # "mock" | "anthropic" | "openai_compat"
api_key_enc = "<encrypted>"    # AES-GCM; empty → fall back to ANTHROPIC_API_KEY / OPENAI_API_KEY env var. Unused by "mock".
model = "claude-sonnet-4-7-20260101"   # placeholder
max_tokens = 4096
temperature = 0.0
endpoint = ""                  # optional override

[code_daemon]
mode = "spawn"                 # "spawn" | "external"
binary = "libre-cr-code"              # path; spawn mode launches this with `mcp-stdio`
external_socket = ""           # for mode = "external"
restart_on_failure = true
max_restarts_per_hour = 5

[mcp_server]
enabled = true
stdio = true                   # libre-cr-review mcp-stdio works
sse = true                     # /mcp on HTTP listener

[global_instructions]
text = ""                      # prepended to system prompt every turn

[limits]
max_tool_turns = 25
max_history_messages = 30
session_idle_evict_days = 90
```

## Configuration UI

The extension does **not** edit provider config or store the API key. Two reasons: (a) the LLM call happens in the daemon, so the key belongs there; (b) the extension's options page is a leaky abstraction across multiple PR-review surfaces.

The daemon serves a minimal config UI at `http://127.0.0.1:<port>/config-ui` — a self-contained static HTML page, no templating. It reads its bearer token from the `?token=` query parameter (the wrapper's `libre-cr config` and the extension popup both open the URL with the token already appended) and attaches it as `Authorization: Bearer` on the JSON calls it makes. The token only ever appears in the URL the user already trusts to launch the daemon; it is never baked into stored markup.

The page lets the user set provider kind, model, max tokens, temperature, endpoint, and API key (posted to `POST /v1/config`). Two conveniences ride the supporting routes:

- A **"Fetch models"** button calls `POST /v1/provider/models` with the in-progress provider+key and populates a model dropdown (for `anthropic`), so the user can pick a model rather than typing an id. The free-text model field stays the source of truth; the dropdown just copies into it.
- On load it calls `GET /v1/provider/detected` and shows a hint when an ambient credential is available ("`ANTHROPIC_API_KEY` detected — leave the key blank to use it").

Pairing flow (first run):

1. User installs extension and runs the daemon via `libre-cr start` (instructions in `08-distribution.md`).
2. The user runs `libre-cr pair`, which issues a one-time code through the running daemon (`POST /v1/pair/issue`).
3. Extension's options page accepts the pairing code (typed, or pre-filled from a pairing deep-link). On submit it hits `POST /v1/pair` with the code and its origin, and receives `{ token, extension_origin }`.
4. Daemon persists the extension's origin to `review.toml` (diagnostics only — CORS does not key on it). Subsequent requests are authenticated by the bearer token alone; the recorded origin is never enforced.

## Concurrency and Cancellation

- Each WS connection is a `tokio` task. Many can be open concurrently (different PRs or follow-up questions).
- A single PR session can have at most one in-flight `ask`. Concurrent attempts on the same session return `409 Conflict`. (We could queue them, but for code review the extra UX complexity isn't worth it; reviewer can wait.) The single-flight claim is an RAII drop-guard, so a failed WS upgrade or a handler panic releases the session instead of leaving it wedged at `409` until restart.
- Cancellation paths:
  - Client closes WS → server aborts.
  - User closes the PR tab → extension closes WS → server aborts.
  - Daemon shutdown → all in-flight turns marked `cancelled`, transaction-committed, then process exits.

## Error Handling

Error responses include a machine-readable code and a human-readable message:

```json
{ "error": "code_daemon_unavailable", "message": "Code intelligence is currently unavailable. Try again in a moment.", "recoverable": true }
```

Categories:

| Code | Meaning | Extension reaction |
|---|---|---|
| `unauthorized` | Bad/missing token | Re-pair prompt |
| `origin_rejected` | reserved (origin checks removed; CORS is `*`) | — |
| `code_daemon_unavailable` | Child crashed, restart pending | Show banner, allow retry |
| `provider_unauthorized` | LLM key bad | Link to config UI |
| `provider_rate_limited` | 429 | Show toast, retry after delay |
| `provider_timeout` | 120 s elapsed | Allow retry |
| `worktree_pending` | Worktree not ready yet | Spinner; auto-retry |
| `worktree_failed` | Fetch failed (e.g., private PR) | Show specific error |
| `validation_failed` | Bad payload from extension | Bug; log and surface |
| `internal` | Unhandled | Show "something went wrong"; log includes a trace ID |

## Logging and Telemetry

- Structured logs via `tracing`. Same defaults as the code daemon.
- Per-turn span: `session_id`, `verb`, `provider`, `model`, `input_tokens`, `output_tokens`, `tool_call_count`, `wall_ms`, `result`.
- **No outbound telemetry.** Logs are local. Sampling and aggregation is on-disk and the user can opt into a planned "anonymized usage stats" report later.

## What This Daemon Does Not Do

- It does not edit code.
- It does not post to GitHub. Export goes to clipboard or to a structured response the extension copies into the GitHub composer.
- It does not run CI, lint, or tests. If we want test-related answers, we use the code daemon's git/AST tools to inspect tests, not to run them.
- It does not learn or fine-tune. The conversation history is for the user, not training.
- It does not share data between users. There is no server-side anything.
