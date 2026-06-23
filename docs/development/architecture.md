# Architecture (as built)

This page describes the implementation. Design rationale lives in
[`specs/02-architecture.md`](../../specs/02-architecture.md); the
round-2 audit ([`REVIEW/round2/03-architecture.md`](../../REVIEW/round2/03-architecture.md))
is the best companion read. Where the audit's findings have since been fixed,
this page says so.

## Three components, two wire protocols

```
┌─────────────────────┐   HTTP + WebSocket          ┌─────────────────────┐
│ Browser extension    │   127.0.0.1:<port>          │ Review daemon        │
│ (extension/)         │   bearer token, CORS-pinned │ (libre-cr-review)    │
│  • PR scrape         │◄───────────────────────────►│  • agent loop        │
│  • selection UI      │   frames: ws_frames.rs ⇄    │  • LLM providers     │
│  • Q&A panel         │           frames.ts         │  • verbs, sessions   │
│  • presentation FX   │                             │  • SQLite store      │
└─────────────────────┘                             └──────────┬──────────┘
                                                               │ MCP (JSON-RPC
                                                               │ over stdio)
                                                               ▼
                          spawns & supervises       ┌─────────────────────┐
┌─────────────────────┐  ┌────────────────────┐    │ Code daemon          │
│ Wrapper CLI          │  │                    │    │ (libre-cr-code)      │
│ (libre-cr)           ├──► libre-cr-review    │    │  • repo registry     │
│  start/stop/pair/    │  │  (5 restarts/60 s) │    │  • worktrees         │
│  doctor/logs/update  │  └────────────────────┘    │  • git/grep/fs tools │
└─────────────────────┘                             └──────────┬──────────┘
                                                               ▼
                                                     local git repos + worktrees
```

- Extension ⇄ review daemon: HTTP routes under `/v1/*`
  (`crates/libre-cr-review/src/server/routes.rs`) + one WS endpoint per ask
  (`server/ws.rs`). Auth: bearer token issued via pairing
  (`server/auth.rs`, `pairing.rs`).
- Review daemon ⇄ code daemon: MCP over stdio. Client actor:
  `crates/libre-cr-review/src/code_daemon/client.rs` (oneshot-per-call;
  pending calls fail with `CodeDaemonUnavailable` on disconnect). Server:
  `crates/libre-cr-code/src/mcp/server.rs` (also `mcp-socket` mode for
  external clients).
- The review daemon's own external MCP surface (`ask_about_pr`) is **Phase 4,
  not implemented** — `libre-cr-review mcp-stdio` prints "not implemented"
  (`crates/libre-cr-review/src/cli.rs`).

## The ownership invariant

**The review daemon never touches a repo filesystem; the code daemon knows
nothing about PRs, sessions, or LLMs.** This is the load-bearing boundary the
audit certified intact. It shows up as an argument-shape discipline:

- Every code-daemon tool takes `repo_path` / `repo_id` / `ref`
  (`crates/libre-cr-code/src/tools/`). No tool takes a `session_id`. PR
  strings appear in that crate only as opaque ref fixtures in tests.
- Every review-daemon internal tool (`get_pr_diff`, `add_note`, …) is keyed
  by session (`crates/libre-cr-review/src/tools/internal.rs`).
- Worktree orchestration is pure MCP: `discover_repo` → `prepare_worktree`
  through the `CodeDaemonClient` trait (`crates/libre-cr-review/src/worktree.rs`);
  the resulting path is stored as an opaque string on the session row.
- The dispatch point is `ToolRouter` (`crates/libre-cr-review/src/tools/router.rs`),
  which fans a tool call to exactly one of three categories: code-daemon
  (forwarded over MCP, `repo_path` injected), internal (session-scoped), or
  presentation (routed back to the extension over the active WS).

**Don't** add a tool that needs both a `session_id` and a `repo_path` — that
is the smell that the boundary is being crossed. PR-awareness goes in the
review daemon; filesystem access goes in the code daemon.

## Crate dependency graph

```
libre-cr-common  ◄──  libre-cr-code
       ▲         ◄──  libre-cr-review
       └─────────◄──  libre-cr-cli        (spawns binaries; depends on no daemon crate)

libre-cr-e2e: dev-deps on libre-cr-common + libre-cr-review (lib, for mock
              scripting types) — spawns real binaries for everything else.
```

`common` is the only crate shared by both daemons — by design it contains
nothing but wire contracts. Keep it that way.

## Where each wire contract is defined

| Contract | Source of truth | Mirror / consumer |
|---|---|---|
| WS frames (`text_delta`, `tool_call`, `presentation_call`, `done`, `error`; `AskInit`, `presentation_result`) | `crates/libre-cr-common/src/ws_frames.rs` | `extension/utils/daemon/frames.ts` (hand-mirrored, runtime-validated by `parseServerFrame`, unknown frames dropped not crashed) |
| HTTP response bodies (`HealthResponse`, `CreateSessionResponse`, `SessionSummary`, `VerbDescriptor`, …) | `crates/libre-cr-common/src/http_api.rs` | same `frames.ts`; route handlers return `Json<T>` of these types |
| Error vocabulary | `crates/libre-cr-common/src/error.rs` (`ErrorCategory`, `ErrorEnvelope`) | `frames.ts` `ErrorCategory` union; code daemon's `ErrorCode` (`crates/libre-cr-code/src/error.rs`) serializes to the same strings |
| Selection shapes | `crates/libre-cr-common/src/selection.rs` | `extension/utils/selection.ts` |
| `PROTOCOL_VERSION` | `crates/libre-cr-common/src/version.rs` (currently `1`) | reported in `GET /v1/health`; checked softly in `extension/utils/daemon/protocol.ts` (console warning + `ui.protocol_mismatch`, never a hard failure) |
| MCP tool result envelope `{ok, error?, message?, …}` | **convention only** — produced by `ensure_ok_envelope` in `crates/libre-cr-code/src/mcp/server.rs`, unwrapped best-effort in `crates/libre-cr-review/src/code_daemon/client.rs` | no shared Rust type exists; callers duck-type `.get("ok")` |

> **Audit status.** The audit's top two erosion risks — untyped `json!` route
> bodies and the `invalid_input` error-string mismatch — have been fixed
> (typed structs in `http_api.rs`; `ErrorCode::ValidationFailed` now matches
> `ErrorCategory`). The MCP envelope remains convention-not-contract; a
> shared envelope type in `common` is the obvious next hardening step.

## Supervision topology and failure handling

Exactly two supervision layers, no double supervision:

1. **Wrapper → review daemon.** `crates/libre-cr-cli/src/supervisor.rs`:
   restart up to 5 times in a 60 s sliding window, then give up. PID file +
   stale-PID handling in `crates/libre-cr-cli/src/proc.rs`.
2. **Review daemon → code daemon.** `crates/libre-cr-review/src/code_daemon/client.rs`:
   reconnect with backoff under a per-hour restart budget
   (`code_daemon.max_restarts_per_hour`, default 5). In-flight calls fail
   with `CodeDaemonUnavailable`; the `ToolRouter` converts that to a
   `ToolOutcome{ok:false}` fed back to the LLM, so **the turn survives a
   code-daemon crash**.

Failure matrix (verified by the audit and by `crates/libre-cr-e2e/tests/spawned_smoke.rs`):

| Failure | Behavior |
|---|---|
| Code daemon dies mid-call | call errors, daemon restarted, turn continues |
| Review daemon dies mid-stream | wrapper restarts it; in-flight turn lost (turns persist at end-of-turn); extension shows a connection error |
| Review daemon SIGKILLed | spawned code daemon sees stdin EOF and `run_stdio` exits — no zombies (`crates/libre-cr-code/src/mcp/server.rs`) |
| Extension torn down mid-ask | WS reader ends; partial answer persisted (`server/ws.rs`) |
| Wrapper SIGKILLed | **known gap**: review daemon survives unsupervised; `libre-cr start` sees the live PID and reports "already running" without re-establishing supervision |

Single-flight per session is enforced by an RAII `BusyGuard` in
`crates/libre-cr-review/src/server/ws.rs` (the audit's leak-on-failed-upgrade
is fixed). It still lives in the WS transport layer — the future Phase-4 MCP
`ask_about_pr` entry point must be routed through the same guard.

## State ownership

One writer per datum. If you find yourself writing someone else's state,
stop.

| State | Owner | Location |
|---|---|---|
| Sessions, turns (incl. notes), tool traces, FTS index | review daemon | `~/.local/share/libre-cr-review/state.db` (`crates/libre-cr-review/src/storage/migrations.rs`: `sessions`, `turns`, `tool_traces`, `turns_fts`) |
| Repo registry, remotes, worktrees | code daemon | `~/.local/share/libre-cr-code/state.db` (`crates/libre-cr-code/src/repo/registry.rs`: `repos`, `repo_remotes`, `worktrees`) + `repos/`, `worktrees/` dirs |
| Token, endpoint, install key, `review.toml`, `code.toml` | review daemon writes token/endpoint/key; humans + config UI write the tomls | `~/.config/libre-cr/` |
| PID file, supervisor log | wrapper | `crates/libre-cr-cli/src/paths.rs` |
| Pairing token, panel geometry, per-session UI flags | extension | `chrome.storage` keys typed in `extension/utils/daemon/storage.ts` |

One sanctioned cross-owner **cache**: `sessions.worktree_path` is the review
daemon's copy of a code-daemon-owned resource. `worktree_ready` is computed
as `worktree_path.is_some()` (`server/routes.rs`) with **no invalidation** —
when worktree LRU eviction ships, this becomes a live bug. If you touch this
area, re-validate via an idempotent `prepare_worktree` instead of trusting
the stored path.

## The Phase-C LSP seam

Code-intel tools dispatch through the `Tool` trait + `ToolRegistry`
(`crates/libre-cr-code/src/tools/registry.rs`), keyed by name. The
tree-sitter-backed tools (`ast_search`, `list_symbols`, `find_definition`,
`find_references`) are **registered today as stubs** with their final names
and schemas (`crates/libre-cr-code/src/tools/stubs.rs`), returning
`unsupported_language`. `crates/libre-cr-code/src/treesitter.rs` is the
grammar seam (`has_grammar` currently always `false`).

To use the seam: implement the real tool (tree-sitter in Phase 1.1, LSP in
Phase C — or an impl that picks a backend internally per language/config) and
swap it in at `build_registry` (`crates/libre-cr-code/src/tools/mod.rs`).
Nothing wire-visible changes; the review daemon and extension are untouched.
See [extending.md](extending.md).

## Erosion risks — contributor guardrails

From the audit, updated to current code. The first block is **fixed — keep it
that way**; the second is **still open — don't make it worse**.

Fixed since the audit (regression here should fail review):
- HTTP bodies are typed in `libre-cr-common/src/http_api.rs`. **Don't** add a
  route returning a `json!` literal (a few trivial `{"ok":true}` acks remain).
- Error strings unified on `ErrorCategory`. **Don't** invent error-code
  strings outside `common::ErrorCategory` / `frames.ts`'s union.
- Both SQLite stores use versioned, transactional, forward-refusing
  migrations. **Don't** sneak schema changes in as `CREATE IF NOT EXISTS`.
- `PROTOCOL_VERSION` is reported and soft-checked. Bump it on breaking wire
  changes (see [conventions.md](conventions.md)).
- Tool dispatch within a turn is parallel (`join_all` in
  `crates/libre-cr-review/src/agent/loop_.rs`); providers hot-swap via
  `ProviderHandle`; `/v1/health` reports real code-daemon state through the
  health hook.

Still open (tracked, intentional for now):
- **GitHub is hardwired in the review daemon's store layer** —
  `parse_pr_url` and the `pr_owner`/`pr_repo` columns
  (`crates/libre-cr-review/src/storage/store.rs`), GitHub pseudo-refs in
  `worktree.rs`, and the extension imports `utils/github/*` directly with no
  platform-adapter indirection. **Don't** deepen this; if you touch URL
  parsing or ref construction, extracting a `platform` module is the wanted
  shape (audit risk #3/#4).
- **MCP result envelope is a string convention** (`ensure_ok_envelope` /
  best-effort unwrap). **Don't** rely on new envelope fields without first
  giving the envelope a shared type in `common`.
- **Silent mock fallback in production `serve`** (`crates/libre-cr-review/src/cli.rs`):
  a missing code binary degrades to `MockCodeDaemonClient` with only a log
  warning. Be suspicious of "it works" when code-intel answers look canned.
- **`sessions.worktree_path` has no invalidation** (above).
- **`libre-cr-e2e` imports review-daemon internals** (`config::ScriptedEvent`,
  `SpawnedClient`) as a lib dev-dep. **Don't** reach deeper into daemon
  internals from tests; these types should eventually move to an explicit
  test-support module.
- **`mock.code_intel` short-circuits inside the production session route**
  (`server/routes.rs`). Test plumbing in a prod handler — contain it, don't
  copy the pattern.
