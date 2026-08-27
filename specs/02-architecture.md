# Architecture

## System Diagram

```
┌──────────────────────────────────────────────────────────────────┐
│                         User's machine                            │
│                                                                   │
│  ┌─────────────────────────┐                                      │
│  │  Browser                 │                                      │
│  │  ┌────────────────────┐  │                                      │
│  │  │ Browser extension   │  │   HTTP + WebSocket                  │
│  │  │ • PR scrape         │  │   localhost:<port>                  │
│  │  │ • Selection UI      │  │   bearer token                      │
│  │  │ • Q&A panel         │──┼─────────────────────────┐           │
│  │  │ • Floating widgets  │  │                         │           │
│  │  └────────────────────┘  │                         │           │
│  └─────────────────────────┘                          ▼           │
│                                            ┌────────────────────┐ │
│                                            │  Review daemon     │ │
│                                            │  (libre-cr-review) │ │
│                                            │                    │ │
│                                            │  • Agent loop      │ │
│                                            │  • LLM provider    │ │
│                                            │  • Verb prompts    │ │
│                                            │  • Per-PR state    │ │
│                                            │  • SQLite store    │ │
│                                            │  • Worktree mgr    │ │
│                                            │                    │ │
│                                            │  Also: MCP server  │◄──── external MCP
│                                            │    ask_about_pr,   │     clients
│                                            │    list_sessions   │     (Claude Code,
│                                            │                    │      Desktop, …)
│                                            └─────────┬──────────┘ │
│                                                      │            │
│                                                      │  MCP       │
│                                                      │  (stdio)   │
│                                                      ▼            │
│                                            ┌────────────────────┐ │
│                                            │  Code daemon       │◄──── external MCP
│                                            │  (libre-cr-code)   │     clients
│                                            │                    │     (standalone use)
│                                            │  • find_symbol     │ │
│                                            │  • find_references │ │
│                                            │  • grep            │ │
│                                            │  • git_log/blame   │ │
│                                            │  • read_file       │ │
│                                            │  • discover_repo   │ │
│                                            │  • prepare_worktree│ │
│                                            │                    │ │
│                                            │  Phase C: LSP      │ │
│                                            │  backend (opt-in)  │ │
│                                            └─────────┬──────────┘ │
│                                                      │            │
│                                                      ▼            │
│                                            ┌────────────────────┐ │
│                                            │  Local git repos   │ │
│                                            │  Worktrees         │ │
│                                            └────────────────────┘ │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
                                              │
                                              │  HTTPS
                                              ▼
                                    ┌──────────────────┐
                                    │  LLM provider    │
                                    │  (Anthropic,     │
                                    │   OpenAI, etc.)  │
                                    └──────────────────┘
```

## Components

### Browser extension

- Runs on GitHub PR pages (Manifest V3 content script + service worker).
- Detects PR context and scrapes PR data from the DOM (carries over from POC).
- Mounts a Shadow-DOM-isolated UI shell with floating widgets.
- Captures user selections in the diff (lines, ranges, files).
- Sends questions + selection context to the review daemon over HTTP/WebSocket.
- Renders streamed answers and persists nothing locally — all state lives in the daemon.

### Review daemon (`libre-cr-review`)

- A long-running Rust process. Started on first use, supervised, listens on a localhost port.
- Owns the LLM client and provider config.
- Owns the agent loop — runs LLM ↔ tool turns until an answer is produced.
- Owns per-PR conversation state, persisted in SQLite.
- Orchestrates worktree readiness *per session* by calling the code daemon's `discover_repo` and `prepare_worktree` tools — does not implement repo discovery or `git worktree add` itself. The resulting worktree path is recorded on the session row and surfaced to the extension via `worktree_ready` / `repo_local_path`. Eviction and disk-side worktree lifecycle are the code daemon's job.
- Acts as an MCP **client** to the code daemon (spawns it as a child process by default, can also connect to a user-managed instance).
- Acts as an HTTP/WS **server** for the browser extension.
- Acts as an MCP **server** itself, exposing high-level tools like `ask_about_pr` for external AI clients.

### Code daemon (`libre-cr-code`)

- A long-running Rust process exposing an MCP server over stdio (default) or local socket.
- Owns code intelligence: AST queries (tree-sitter), structural search (ast-grep), text search (ripgrep), git operations (gitoxide).
- Owns repo discovery and worktree primitives — finding which local checkout matches a remote URL, creating/cleaning worktrees for arbitrary refs.
- Knows nothing about PRs, LLMs, or conversations. Every tool takes a `repo_path` argument.
- Standalone product: usable by any MCP client (Claude Code, Claude Desktop, custom agents) without ever installing the review daemon or the browser extension.

## Ownership Boundary

The split between code daemon and review daemon is enforced by the kinds of arguments each component's tools accept.

| Code daemon owns | Review daemon owns |
|---|---|
| Tools that take `repo_path` and ref/file/symbol arguments | Tools and APIs that take `pr_id` or `session_id` arguments |
| Local checkouts, worktrees, indexed code structure | PR identity, scraped PR data, conversation history |
| Stateless and ref-aware operations (find this symbol on this commit) | Stateful, conversational operations |
| MCP tools only | HTTP API for extension + MCP server for external clients |
| Could be replaced by a third-party MCP server with the same tool surface | Specific to this product |

If a tool needs a `pr_id`, it belongs in the review daemon. If a tool needs only a `repo_path` + git ref, it belongs in the code daemon.

## Data Flow: Asking a Question

```
1. User opens PR in browser
   ─→ Extension scrapes PR context (owner/repo/number/diff/comments)
   ─→ Extension POSTs to review daemon: /sessions/<pr-id>/init
   ─→ Review daemon:
        • Looks up or creates a session row in SQLite
        • Calls code daemon: discover_repo(remote_url)
        • Calls code daemon: prepare_worktree(repo_path, pr_ref)
        • Returns session info to extension (worktree ready)

2. User selects code in diff, clicks a verb or types a question
   ─→ Extension opens WebSocket: /sessions/<pr-id>/ask
   ─→ Sends: { question, selection, verb? }
   ─→ Review daemon:
        • Loads conversation history for this session
        • Resolves verb (if any) to a prompt template
        • Composes system prompt + history + selection context + question
        • Starts agent loop:
            loop:
              LLM call (with code-daemon tools, internal tools, and
                         presentation tools all registered)
              if response is final text: stream to extension, exit loop
              for each tool_use:
                dispatch by category:
                  ─ code-daemon tools → MCP call to code daemon
                  ─ internal tools    → direct Rust call (PR/session data)
                  ─ presentation tools→ presentation_call frame on the WS,
                                        extension executes, replies
                append result to messages
        • Persist new turn (question, all tool calls, final answer) to SQLite

3. Extension streams the answer into the Q&A panel
   (Streaming surfaces three frame kinds: text_delta, tool_call/result,
    and presentation_call/result. The extension renders presentation calls
    immediately — highlighting lines, scrolling, etc. See 09-presentation-tools.md.)
   ─→ User can ask follow-up questions in the same panel (same session)
   ─→ Conversation accumulates per-PR

4. When user is done reviewing:
   ─→ Extension calls /sessions/<pr-id>/export
   ─→ Review daemon returns a Markdown draft assembled from the conversation
   ─→ Extension copies to clipboard / opens GitHub review composer for the user to paste
```

## Data Flow: External MCP Client

```
Claude Desktop or Claude Code connects to review daemon's MCP server.

Tools exposed:
  • ask_about_pr(pr_url, question, selection?) → string
  • list_sessions() → [{ pr_url, last_active }]
  • get_session_notes(pr_url) → markdown

Tools NOT exposed at the review-daemon MCP layer:
  • Anything that bypasses the agent loop (the value here is the prompts + verbs)

To use the code daemon directly with no review-daemon involvement:
  External client connects to libre-cr-code MCP server, calls its tools
  with repo_path arguments. Fully standalone path.
```

## Transports

### Extension ↔ review daemon: localhost HTTP + WebSocket

- Daemon binds 127.0.0.1 on a port read from a config file (default ephemeral, written on startup).
- Token authentication: daemon writes a 256-bit bearer token to `~/.config/libre-cr/token` (mode 0600). Extension reads it via the WXT options page on first use, stores it in `browser.storage.local`. Token is included as `Authorization: Bearer <token>` on every request.
- Origin check: daemon rejects requests whose `Origin` header is not from a configured browser extension origin (set during the extension's first-run setup).
- CORS: permissive (`*`). The bearer token is the boundary; content-script requests carry the page origin under MV3, so an extension-origin allowlist would block the product.
- One-shot requests use HTTP POST (init, export, list sessions). Streaming responses (ask) use WebSocket.
- The WebSocket carries two distinct directions of streaming during a turn:
  - **Daemon → extension:** `text_delta`, `tool_call`, `tool_result`, `presentation_call`, `done`, `error`.
  - **Extension → daemon:** initial `{question, selection, verb}` frame, then `presentation_result` frames in response to each `presentation_call`.
- See `04-review-daemon.md` for the HTTP API surface and `09-presentation-tools.md` for the presentation frame protocol.
- The daemon also serves a self-contained configuration web page at `GET /config-ui` (static HTML, token passed via `?token=`) and supporting JSON routes the page consumes: `POST /v1/provider/models` (fetch a candidate provider's live model list) and `GET /v1/provider/detected` (report which ambient env-var keys are available). This page is opened by the wrapper's `libre-cr config` and by the extension popup's "Configure daemon" link, not embedded in the extension. CORS is dynamic: the allowlist is read on every request from the live extension-origin value, so an origin learned during pairing takes effect without a daemon restart.

### Review daemon ↔ code daemon: MCP over stdio

- Review daemon spawns code daemon as a child process by default. stdio MCP — no port, no auth.
- Power user mode: review daemon can be configured to connect to an existing code daemon over a Unix socket. Useful when the code daemon is run independently (e.g., for use with Claude Desktop) and the review daemon should reuse it.
- Tool schemas defined by the code daemon; review daemon dynamically discovers and registers them at startup.

### Typed HTTP wire contract

The review daemon's HTTP response bodies are defined as `Serialize`/`Deserialize` structs in `libre-cr-common` (`http_api.rs`) — `HealthResponse`, `CreateSessionResponse`, `SessionSummary`, `PairIssueResponse`, `PairRedeemResponse`, `SearchResponse`, `ExportResponse`, `ModelsResponse`, `DetectedCredentials`, `VerbDescriptor`, and friends. The Rust side is the source of truth; the extension's `frames.ts` mirrors these shapes. Field renames are breaking changes. This replaces the earlier pattern of building responses from ad-hoc `json!` literals, which had let the Rust and TS shapes drift independently.

The shared crate also defines `PROTOCOL_VERSION` (currently `1`). The daemon reports it in `GET /v1/health` (`protocol_version`); the extension carries a matching constant and does a soft version check on each session init (a mismatch logs a warning and is surfaced in the Options diagnostics, but never blocks).

### Review daemon ↔ external MCP client: MCP over stdio or SSE

- stdio for child-process use (`claude` CLI, etc.).
- Optional SSE endpoint on the same HTTP listener for clients that prefer it.

### Code daemon ↔ external MCP client: MCP over stdio

- Same. The code daemon is just another MCP server.

## State Locations

| State | Lives in | Notes |
|---|---|---|
| LLM provider config + API key | Review daemon's config file (encrypted at rest) | User edits via the daemon's config UI at `/config-ui`; extension does NOT see the key. If no key is saved, the daemon falls back to `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` from its environment |
| Bearer token for extension↔daemon | `~/.config/libre-cr/token` (mode 0600) | Generated on first daemon start |
| Per-PR conversation history | Review daemon's SQLite DB | One row per turn, keyed by `(pr_url, turn_id)` |
| Investigation verb definitions | Review daemon's source code (Rust) | Phase B: hardcoded. Plugin model deferred. |
| Repo registry (remote URL → local path) | Code daemon's SQLite DB | Populated by repo scans + manual config |
| PR worktrees | Code daemon's managed dir: `~/.local/share/libre-cr-code/worktrees/<owner>/<repo>/pr-<n>` | LRU eviction at configurable threshold |
| User notes (added manually in Q&A panel) | Review daemon's SQLite DB, alongside conversation | Treated as just-another-turn for export purposes |
| Browser extension state | `browser.storage.local` | Only the daemon URL and the bearer token; everything else lives server-side |

## Failure Modes And How They Surface

- **Code daemon not running.** Review daemon detects this on every tool call, restarts the child process up to N times, then surfaces an error to the extension ("Code intelligence unavailable — restart the daemon"). User-facing.
- **Review daemon not running.** Extension cannot reach `localhost:<port>`. Shows a banner: "Review daemon not running. [Start daemon] [Configure]." The "Start daemon" button is best-effort (we cannot launch arbitrary binaries from a content script; user must run the install command or click a tray icon — see `08-distribution.md`).
- **LLM provider rate-limited or down.** Review daemon's agent loop returns an error event to the WebSocket. Q&A panel shows the error with a retry button. Conversation state is preserved.
- **Worktree fetch fails (network, auth).** Code daemon surfaces error to review daemon, which surfaces to extension. Q&A panel offers to retry or fall back to "read-only, current branch state" mode (uses whatever's checked out, even if not the PR ref).
- **No local checkout for the PR's repo.** Code daemon prompts to clone (via review daemon → extension UI). User confirms; code daemon clones into its managed location. This flow is deliberately friction-bearing because cloning is not free.

## Why This Shape

- **Two daemons, not one** — because code intelligence is independently valuable and worth shipping as a standalone product. Bundling everything into one daemon would couple our PR-review iteration speed to the code-daemon's stability story, and vice versa.
- **Review daemon as MCP server** — because the same agent loop is useful to other AI clients (Claude Code, Desktop). We get a cheap second distribution surface without designing a separate "API."
- **Localhost HTTP, not Native Messaging** — because the same daemon serves multiple browsers / non-browser clients (CLI, future IDE plugins). Native Messaging is per-browser and brittle.
- **Bearer token, not anonymous** — because any local process can hit localhost. Anonymous localhost endpoints are a known foot-gun (any visited webpage can probe them via `fetch`).
- **SQLite for state** — durable, queryable, debuggable. Conversation history is genuinely useful as a queryable artifact ("when did I last review this file?").
- **Worktrees, not branch-switching** — never touch the user's active working tree. Worktrees are git's built-in answer for "I want a different ref checked out somewhere else without disturbing what I'm doing."
