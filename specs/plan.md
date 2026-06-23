# Libre CR — Implementation Plan

## Context

This is a ground-up rebuild informed by a working POC (`libre-cr-assistant`) that taught us:

1. **LLM-as-orchestrator was the wrong model.** Code review is human-driven; the assistant answers questions rather than auto-annotating.
2. **Browser-only was the wrong surface.** Real review needs full repo access, not just the visible diff.

The new architecture is three components: a Rust **code daemon** (`libre-cr-code`) exposing repo intelligence over MCP, a Rust **review daemon** (`libre-cr-review`) running the agent loop and persisting conversations, and a thin **browser extension** that captures selections and renders streamed answers. The code daemon is independently valuable and shipped as its own product.

This plan delivers that as **Phase B** (no LSP), with **Phase C** (LSP integration) explicitly designed for but deferred. The code daemon's tool layer is built so LSP backends slot in behind the same MCP surface.

## Goals

- **First demo milestone:** Open a real GitHub PR, ask "find callers of this function" via the extension, receive a streamed answer grounded in the user's local checkout. This validates the full path: scrape → pair → worktree → tool dispatch → agent loop → render.
- **First usable milestone:** All five Phase-B verbs work end-to-end. Conversation persists. Notes can be added and exported as a Markdown draft.
- **Standalone code daemon milestone:** `libre-cr-code` works against Claude Code / Claude Desktop with its tools, with zero involvement from the review daemon. Shipped to homebrew/scoop independently.

## Phased Plan

### Phase 0 — Repo scaffolding [S]

Set up the three workspaces, CI, common types, build pipeline.

**Tasks**

1. Cargo workspace at repo root with four members: `crates/libre-cr-code`, `crates/libre-cr-review`, `crates/libre-cr-cli`, `crates/libre-cr-common`. All crates and binaries follow the uniform `libre-cr-*` naming; the wrapper's binary is the only exception, exposed as `libre-cr` because that's what users type constantly.

   | Crate | Binary | Notes |
   |---|---|---|
   | `libre-cr-code` | `libre-cr-code` | Code daemon (MCP server, standalone product) |
   | `libre-cr-review` | `libre-cr-review` | Review daemon |
   | `libre-cr-cli` | `libre-cr` | Wrapper CLI (binary name is short by intent) |
   | `libre-cr-common` | — | Shared types: MCP wire types, message types, error categories |
2. CI matrix: macOS, Linux, Windows × stable Rust. Format/clippy/test. Cross-compile to all five release targets.
3. Browser extension package under `extension/` — fresh WXT init, identical structure to POC for ease of porting later.
4. License (MIT, carry over from POC), README, CONTRIBUTING stub.
5. Release workflow stub (signed artifacts, version bump script).

**Verify:** `cargo build --workspace` succeeds locally and on CI. Extension builds. Release workflow runs in dry-run mode.

---

### Phase 1 — Code daemon: MVP tools [M]

> Independent of Phase 2.

Build `libre-cr-code` with the Phase-B tool surface, MCP server, and worktree management. No browser, no agent, no LLM.

**Tasks**

1. MCP server skeleton (`rmcp` or hand-rolled). stdio transport. `tools/list`, `tools/call`. Tool trait + dispatcher (per `03-code-daemon.md`).
2. Repo registry: SQLite schema, canonical remote URL handling, `discover_repo` and `scan_for_repos` tools.
3. Git backend (`gitoxide` + `git` CLI fallback): `git_log`, `git_blame`, `git_show`, `git_diff`.
4. Worktree management: `prepare_worktree`, `list_worktrees`, `remove_worktree`. LRU eviction background task.
5. Filesystem reads: `read_file`, `list_dir`, `stat_file`. All ref-aware.
6. Search backends:
   - ripgrep via the `grep` crate → `grep` tool.
   - ast-grep via `ast_grep_core` → `ast_search` tool.
   - tree-sitter with grammars for Rust, Go, TS, JS, Python, Java, C, C++, Ruby, PHP, **Bash**. See `03-code-daemon.md` § Language Support.
7. AST-derived symbol tools: `list_symbols`, `find_definition`, `find_references` (Phase-B implementations). Name-based, not semantic; results include a `confidence` field.
8. `clone_repo` for the fallback path when discovery misses.
9. Config file loading, log infrastructure (`tracing`), CLI subcommands (`mcp-stdio`, `mcp-socket`, `scan`, `discover`, `tools`, `doctor`).
10. Integration tests: fixture repos in `tests/fixtures/`. Test every tool against deterministic inputs.

**Verify:** Manually point Claude Desktop at `libre-cr-code mcp-stdio`. Run `scan_for_repos`, `find_references`, `git_blame`, `git_log` against a real local repo. Output is correct and useful. This is the **standalone code daemon milestone**.

**Key crates:** `rmcp` (or hand-rolled MCP), `gitoxide`, `grep` (ripgrep), `ast_grep_core`, `tree-sitter`, `tree-sitter-*` grammars, `tokio`, `tracing`, `rusqlite`, `serde`.

---

### Phase 2 — Review daemon: skeleton + agent loop [M]

> Can develop in parallel with Phase 1 once shared types are stable.

Build `libre-cr-review`'s server, agent loop, and provider clients. No verbs, no extension yet.

**Tasks**

1. HTTP server skeleton (`axum`). Auth middleware (bearer token). CORS allowlist.
2. SQLite schema: `sessions`, `turns`, `tool_traces`, `turns_fts`. Migration framework.
3. Session manager: `POST /v1/sessions`, `GET /v1/sessions/:id`, list, delete.
4. Provider abstraction (Rust trait). Anthropic + OpenAI-compatible implementations, ported in spirit from the POC's TS versions. SSE parsing, retry logic, streaming `StreamEvent` enum.
5. Agent loop (`run_turn`): tool-use loop, message accumulation, frame emission via a `FrameSink` trait. Bounded by `MAX_TOOL_TURNS`.
6. Tool router skeleton (no external code daemon yet — mock tools register a "fake_grep" etc. for testing the loop).
7. WebSocket endpoint `/v1/sessions/:id/ask` (single in-flight per session).
8. Token bootstrap, endpoint file writing, pairing endpoint `/v1/pair`.
9. Provider config encryption (AES-GCM, machine-bound material).
10. Unit tests: mock provider returning canned SSE; verify agent loop correctness, cancellation, error frames, turn persistence.

**Verify:** `curl` against the HTTP API. WS via a small Rust test client. Hardcoded mock tools. Run an agent loop end-to-end, persist a turn, read it back. No code daemon involved yet.

**Key crates:** `axum`, `tokio-tungstenite`, `rusqlite`, `aes-gcm`, `tracing`, `reqwest`, `eventsource-stream` (for SSE).

---

### Phase 3 — Daemon-to-daemon wiring [S]

> Depends on Phases 1 and 2.

Wire the review daemon as an MCP client of the code daemon. Real tools replace the mocks.

**Tasks**

1. MCP client in `libre-cr-review` (the inverse of Phase 1's server). Spawn `libre-cr-code mcp-stdio` as a child process. Discover tools via `tools/list`. Cache schemas.
2. Tool router dispatch: route external-daemon tools through MCP, internal tools (defined in Phase 4) through direct calls.
3. Auto-inject `repo_path` for code-daemon tools that take one. Review daemon knows the session's worktree path.
4. Health monitoring: detect child death, restart with backoff, surface as `code_daemon_unavailable` error.
5. Power-user mode: connect to existing `libre-cr-code mcp-socket` instead of spawning. Config flag.
6. Integration tests: spin up real `libre-cr-code` from `libre-cr-review`, exercise full path.

**Verify:** Agent loop in `libre-cr-review` successfully calls `find_references` on a real repo. Errors surface cleanly when `libre-cr-code` dies mid-call.

---

### Phase 4 — Internal tools + verbs + worktree orchestration [M]

> Depends on Phase 3.

Add the PR-aware internal tools, verb registry, and session-level worktree orchestration.

**Tasks**

1. Internal tools: `get_pr_diff`, `get_pr_comments`, `get_pr_metadata`, `get_selection`, `add_note`, `session_history_search`.
2. Session initialization expansion: when `POST /v1/sessions` arrives, call code daemon's `discover_repo` (and `clone_repo` on miss + user confirmation), then `prepare_worktree`. Reflect readiness in session state.
3. Verb registry: hardcoded structs for `find_callers`, `show_history`, `related_tests`, `compare_to_base`, `explain`. Each with its system-prompt addendum, required selection, suggested tools. Per `06-investigation-verbs.md`.
4. `GET /v1/verbs` endpoint.
5. Notes endpoints: `POST/PATCH/DELETE /v1/sessions/:id/notes`.
6. Tests: end-to-end against a fixture PR + fixture repo. Run `find_callers`, verify the LLM saw the right tool schemas, verify a sensible turn was persisted.

**Verify:** From a Rust test, simulate an extension POSTing a session and WS-asking `find_callers`. The daemon runs the full path: prepares the worktree, the agent uses code-daemon tools, an answer is streamed back. **This is the system-without-UI demo.**

---

### Phase 5 — Browser extension: pairing + scraping + Q&A panel [L]

> Depends on Phase 4.

The user-facing surface. Carry forward the GitHub adapter + shell from the POC; build the new selection UI and Q&A panel.

**Tasks**

1. New WXT extension under `extension/`. Manifest V3. Content script + background + options + popup.
2. Port from POC: `platform/github/` adapter and selectors, `ui/theme.ts`, Shadow DOM shell, `EventBus`, floating widget mechanics (drag/resize/tile/gridMath/etc.), CSP-safe validator.
3. Daemon client TS module: `fetch` + `WebSocket` against `127.0.0.1:<port>`, token auth, typed request/response. Designed to be hostable in either content script or background context.
4. Pairing flow (options page): code submission, deep-link handler. Persist endpoint + token in `browser.storage.local`.
5. Selection capture: line, range, symbol pickers attached to diff tables. Symbol picker uses a minimal tree-sitter-lite or a regex fallback for identifier extraction.
6. Q&A panel as a floating widget: conversation list (Q&A + notes), verb buttons, question input. Streams from WS, renders text deltas + tool call/result frames.
7. Diff-side overlays: "show on diff" for line references in answers. "Ask" affordance in the gutter on hover.
8. Popup: daemon status, recent sessions list, config link.
9. Options: pairing, theme, diagnostics.
10. Soft warnings for selector mismatches, daemon unreachable, worktree pending.
11. **Presentation tools** (`09-presentation-tools.md`): five tools (`highlight_lines`, `annotate_line`, `scroll_to`, `open_link`, `clear_presentation`) registered in the review daemon's tool router; presentation handler in the extension that listens for `presentation_call` frames on the active WS, executes via the ported POC `UIController` implementations, and replies with `presentation_result`; per-turn effect bookkeeping with `effect_id`s; Clear button in the panel footer; auto-clear-on-new-question default; settings toggles for the three knobs (auto-clear, open_link targets, global disable).

**Verify:** First demo milestone. Open a real GitHub PR with the daemon running. Pair the extension. Select a function. Click `Find callers`. The answer streams into the panel **and the call sites get highlighted in the diff with a "Clear all" footer reflecting the effect count.**

**Key from POC:** `src/platform/github/adapter.ts`, `src/platform/github/selectors.ts`, `src/ui/shell/*` (Shell, Sidebar, Toolbar, FloatingWidget, TilingContainer, gridMath, useDrag, useResize), `src/ui/theme.ts`, `src/utils/event-bus.ts`, `src/utils/validate-params.ts`.

---

### Phase 6 — Export + power features [M]

> Depends on Phase 5.

Make the conversation valuable as an artifact. Polish the rough edges.

**Tasks**

1. Export endpoint and modal: notes-only / notes+context / full transcript. Markdown + GitHub-structured formats.
2. Cross-session FTS search endpoint and popup search.
3. "Save as note" action on assistant turns.
4. Per-PR diff-change detection: when the user reopens a session with a different head ref, show the "PR diff changed" banner.
5. Free-form question parity: same UI, no verb addendum.
6. Conversation timeline polish: collapse old turns, severity icons, edit-in-place notes.
7. Onboarding pass: empty-state messaging, "what is libre-cr" tooltip, first-pair walkthrough.

**Verify:** A reviewer can do an entire PR review in libre-cr, export a Markdown draft, paste it into the GitHub review composer, submit. End-to-end usable flow.

---

### Phase 7 — Wrapper CLI + distribution [M]

> Depends on Phases 1, 2, 5 (binaries + extension package).

Make installation a real experience.

**Tasks**

1. `libre-cr` wrapper crate: start/stop/restart/status/logs/pair/doctor/update/uninstall.
2. Process supervision (5-restart-per-60s policy).
3. Log file management (rotate, retain 14 days, separate streams).
4. Signed-release pipeline: codesign + notarize on macOS, Authenticode on Windows, GPG on Linux. SHA256 manifest. Release manifest endpoint.
5. Homebrew formula + tap.
6. Scoop manifest + bucket.
7. install.sh for Linux.
8. Web Store + Add-ons listing for the extension.
9. `libre-cr doctor`: git presence, port availability, file permissions, code-daemon health.
10. Documentation site (docs/ markdown rendered as static site). Includes the spec files, installation guides, troubleshooting.

**Verify:** A fresh machine can `brew install libre-cr`, run `libre-cr start`, install the extension, pair it, and review a PR.

---

### Phase 8 — Polish, hardening, performance [L]

The post-launch grind. Items here ship as patches; no single milestone.

- Selector resilience: refresh GitHub selectors as needed, add fallbacks.
- Streaming UI: backpressure, partial-response recovery on disconnect.
- Provider error UX: better messages for each common provider failure.
- Performance: tool latency budgets in the spec (`03-code-daemon.md`) — instrument and tune.
- Tests: golden snapshots for verb prompts, integration tests for the agent loop against recorded LLM responses.
- Accessibility: keyboard navigation in Q&A panel, ARIA on dynamic regions.
- Memory: AST cache tuning, worktree eviction thresholds based on real disk usage.

---

### Phase 9 — Optional GitHub posting (post-v2) [M]

> Optional. Ship when there's demand.

OAuth-based posting of structured reviews back to GitHub.

**Tasks**

1. OAuth app registration; redirect flow through the daemon's localhost.
2. Token storage (encrypted) in daemon config.
3. `POST /v1/sessions/:id/post-review` — uses the existing `github_review` export format.
4. Inline-comment posting (one PR review with multiple comments, single submit).
5. UI: "Post to GitHub" button in the export modal.

Held back from v2 because (a) the clipboard workflow is fine, (b) OAuth adds support burden, (c) some teams forbid third-party OAuth.

---

### Phase C — LSP integration [L]

> Independent track. Begins after Phase 4 stabilizes.

Add LSP backends to the code daemon. No new MCP surface; same tools, better results.

**Tasks**

1. LSP backend trait in `libre-cr-code`. Implementations spawn and manage language-specific LSPs as subprocesses.
2. Backend registry: per-language config in `code.toml`, autodetect installed LSPs on PATH (`rust-analyzer`, `gopls`, `typescript-language-server`, `pyright`, etc.).
3. Lifecycle: spawn on first use for a language; shut down after configurable idle timeout.
4. Route `find_definition`, `find_references`, `hover`, `type_at`, `workspace_symbols`, `call_hierarchy` through LSP when available, fall back to AST otherwise.
5. New phase-C tools as documented: `hover`, `type_at`, `workspace_symbols`, `call_hierarchy`.
6. Tests: integration against rust-analyzer in a fixture Rust repo. Smoke tests for each supported language.
7. `libre-cr-code doctor` extended: which LSPs are configured and healthy.
8. Documentation: "configuring LSPs" page in the docs site.

**Verify:** `find_references` against a polymorphic call site returns true references with rust-analyzer running; AST fallback returns approximate ones without it. Both work; the LSP one is better.

**Key crates:** `async-lsp`, `lsp-types`.

---

## Parallelism Map

```
Phase 0 (scaffolding)
   ├── Phase 1 (code daemon MVP) ─┐
   └── Phase 2 (review daemon)    ─┤
                                    ├── Phase 3 (wiring)
                                    │     └── Phase 4 (verbs + internal tools)
                                    │           └── Phase 5 (browser extension) ⭐ first demo
                                    │                 └── Phase 6 (export + polish) ⭐ first usable
                                    │                       └── Phase 7 (distribution)
                                    │                             └── Phase 8 (polish, ongoing)
                                    │                                  └── Phase 9 (GitHub OAuth, optional)
                                    │
                                    └── Phase C (LSP integration, independent)
```

Phases 1 and 2 are the parallelizable trunk. Phases 3–7 are mostly sequential. Phase 8 is a continuous post-launch lane. Phase 9 is opt-in. Phase C is a parallel track that can be developed against Phase 1 alone (the LSP backends slot into `libre-cr-code` without needing the review daemon or extension).

## Decisions Locked

| Decision | Choice |
|---|---|
| Pivot model | Human-driven Q&A, not LLM-orchestrated functions |
| Components | Browser extension + review daemon + code daemon |
| Daemon language | Rust (single static binary) |
| Daemon ↔ extension transport | Localhost HTTP + WebSocket + bearer token |
| Daemon ↔ daemon protocol | MCP (stdio by default, socket optional) |
| Code intelligence (Phase B) | ast-grep + ripgrep + tree-sitter + gitoxide |
| LSP integration | Phase C; designed for in B, deferred to ship |
| Repo binding | Scan configured roots → fall back to clone |
| PR branch state | Silent fetch into managed worktree |
| Killer workflow | Investigation verbs + persistent notes |
| Verbs (Phase B) | `find_callers`, `show_history`, `related_tests`, `compare_to_base`, `explain` |
| Conversation storage | SQLite in review daemon |
| Export format | Markdown clipboard + structured-for-future-OAuth |
| API key location | Review daemon config (extension never sees it) |
| External MCP surface from review daemon | `ask_about_pr`, `list_sessions`, `get_session_history`, `export_session` |
| Presentation tools | Five tools available to the LLM during browser-extension turns; routed back through the WS to the extension. Not exposed via the external MCP surface. |
| IDE integration | Deferred; external MCP clients (Claude Desktop / Code) fill the gap |
| Telemetry | None in v2 |
| Distribution | Brew + Scoop + GitHub Releases; wrapper CLI supervises both daemons |

## Open Items (Tracked, Not Blocking)

- Selector breakage handling — we may want a more structured "report this PR's HTML for our selector update" path beyond the soft warning.
- Eviction defaults — 5 GB worktree threshold and 90-day session retention are guesses; tune from real usage.
- Symbol picker in the extension — minimal tree-sitter port versus regex fallback. Start with regex, upgrade if needed.
- Provider list — should we ship a third built-in (Ollama with native quirks)? OpenAI-compatible covers it for v2; can add later if user requests warrant.
- Worktree creation for private PRs — the daemon needs the user's git credentials. We rely on the user's existing git config (SSH keys, credential helpers). Document this in `08-distribution.md` if it gets surprising.

## Out Of Scope

Restated from `01-overview.md` because it bears repeating against the temptation of scope creep:

- No autonomous review.
- No code editing or rewriting.
- No CI integration.
- No team-shared state.
- No bundled language servers.
- No mobile/iPad.
- No corporate proxy auto-config.
