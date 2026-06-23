# Libre CR v2 — Certification Report

Five-agent review synthesis. Individual reports:

- [`01-rust-core.md`](01-rust-core.md) — Rust correctness, concurrency, spec compliance
- [`02-security.md`](02-security.md) — auth, secrets, path traversal, subprocess
- [`03-tests.md`](03-tests.md) — coverage, fidelity, brittleness
- [`04-frontend.md`](04-frontend.md) — extension TS/React, DOM safety, accessibility
- [`05-distribution-docs.md`](05-distribution-docs.md) — supervisor, CLI, install, docs

## Verdict

**Ship with fixes.** The architecture is sound and the seams hold. The Phase B
v2 scope is feature-complete in code, but the demo path is currently blocked by
two end-to-end gaps that exist *because* every test bypasses them via mocks:
the pairing flow doesn't connect the two CLIs to the running daemon, and both
real LLM providers drop streamed tool inputs. Neither is hard to fix.

160+46 tests passing, fmt clean, clippy `-D warnings` clean. No Critical
security finding. No architectural redesigns needed.

## Critical (block v2 demo)

Demo-stopping bugs. Fix before any external user touches the product.

### From multiple reviewers

**C1 — Pairing is end-to-end broken.** Both `libre-cr pair`
(`crates/libre-cr-cli/src/pair.rs:17`) and `libre-cr-review pair`
(`crates/libre-cr-review/src/cli.rs:71`) generate codes locally. The running
daemon's `PairingStore` (held in `AppState`) never sees them, so the extension's
`POST /v1/pair` always returns 401. Fix: add a token-authenticated
`POST /v1/pair/issue` route and have both CLIs hit it.
*Reported by: Rust core C2, Distribution C1.*

**C2 — `libre-cr config` opens a 404.** Wrapper constructs
`<endpoint>/config-ui` (`crates/libre-cr-cli/src/commands/config.rs:11`); the
daemon serves `/v1/config`, not `/config-ui`. There is also no actual config
*UI* served on any path — only the JSON `/v1/config` API. Fix: add a minimal
`GET /config-ui` HTML page on the daemon (the spec calls for one), and have the
wrapper open it.
*Reported by: Distribution C2 (also relevant: Rust core C3 — see C3 below).*

### Rust correctness

**C3 — `POST /v1/config` mutates in-memory only.** Provider/key changes are
lost on restart (`crates/libre-cr-review/src/server/routes.rs:374`). Fix:
persist to disk on every accepted mutation; thread `cfg_path` through
`AppState`. With C1/C2 fixed this is the lever the user uses to set their API
key — silently losing it is a footgun.
*Reported by: Rust core C3.*

**C4 — Anthropic & OpenAI providers drop streamed tool inputs.**
`parse_anthropic_event` (`crates/libre-cr-review/src/provider/anthropic.rs:152`)
ignores `input_json_delta` fragments. OpenAI's `parse_openai_chunk`
(`provider/openai_compat.rs:195`) emits a fresh `ToolUse` per arguments-chunk
with partial JSON. Any tool-use turn against a real API ships an empty or
garbage `input` to the agent loop. The MockProvider doesn't exercise this
path, so 160 passing tests don't catch it. Fix: buffer fragments per
`block_index`/`tool_call_id` until block-stop, parse once, emit one `ToolUse`.
*Reported by: Rust core C1.*

### Frontend

**C5 — Popup uses `dangerouslySetInnerHTML` for daemon snippets.**
`extension/entrypoints/popup/Popup.tsx:131` does string escaping then sets
inner HTML for search results. Unnecessary — `<span>{snippet}</span>` is safer
and equivalent. Fix: drop the `dangerouslySetInnerHTML`.
*Reported by: Frontend C1.*

**C6 — Presentation effects orphaned on Turbo nav.**
`QaPanel.tsx:40` keeps a `PresentationManager` in a `useRef` with no
`detachAll()` / `clearAll()` on unmount. Spec § Effect Lifecycle requires
clearing on navigation. After navigating between PRs, the previous PR's
highlights stay in the DOM. Fix: invoke `clearAll()` in the `useEffect` cleanup
and wire it to the content script's existing `tearDown`.
*Reported by: Frontend C2.*

**C7 — `AskSession.inflight` is never cleared on early return.**
`extension/utils/daemon/ws.ts` only clears `inflight` in `close()`. If the
caller's `try`/`finally` is bypassed (early throw before WS opens), the session
is permanently locked. Fix: clear `inflight` in the WS `error`/`close` handlers
themselves, not just `close()`.
*Reported by: Frontend C3.*

### Distribution / ops

**C8 — Code-daemon stderr is dropped.**
`crates/libre-cr-review/src/code_daemon/transport.rs:83` pipes `Stdio::null()`.
The spec mandates a `libre-cr-code.log`; today there is none, and
`libre-cr logs` doesn't reference one. Fix: open
`~/.local/state/libre-cr/log/libre-cr-code.log` (append) and pipe child stderr
into it; wire it into `libre-cr logs`.
*Reported by: Distribution C3.*

**C9 — `code.toml` lives in the wrong namespace.** Code daemon uses
`~/.config/libre-cr-code/config.toml` (`crates/libre-cr-code/src/config.rs:116`);
spec § Configuration Layout says `~/.config/libre-cr/code.toml`. Choose one and
fix the other. Recommendation: align code daemon with the spec — wrapper-managed
config tree under a single `~/.config/libre-cr/`.
*Reported by: Distribution C4.*

## Important (should fix before v2 ships)

### Rust correctness

- **I1.** `/v1/health` hardcodes `code_daemon: {connected: true, version: "mock"}` instead of calling the existing `health_hook`. `crates/libre-cr-review/src/server/routes.rs:408`. *(Rust core)*
- **I2.** Tool calls inside a single LLM turn are serialized; spec § Agent Loop requires parallel dispatch. `agent/loop_.rs:269`. *(Rust core)*
- **I3.** Cancelled turns are never persisted; `persist_cancelled` is `#[allow(dead_code)]`. `server/ws.rs:206` + `agent/loop_.rs:332`. *(Rust core + Tests)*
- **I4.** `git fetch` / `git worktree add` use `std::process::Command::output()` from `async fn`; blocks the tokio worker. `crates/libre-cr-code/src/repo/worktree.rs:66`. *(Rust core)*
- **I5.** WS `tokio::select!` borrows a freshly-spawned wrapper instead of the inner task — leaky and structurally odd. `server/ws.rs:206`. *(Rust core)*
- **I6.** Race between `next_ordinal` and `insert_turn` under independent locks — concurrent note POSTs can collide on `UNIQUE(session_id, ordinal)`. `storage/store.rs:160`. *(Rust core)*
- **I7.** Worktree single-flight map grows unbounded; one entry per unique ref ever requested. `crates/libre-cr-code/src/repo/worktree.rs:33`. *(Rust core)*
- **I8.** `prepare_worktree` short-circuits on `.git` existence and never re-fetches force-pushed refs. `worktree.rs:59`. Combined with the `pr_diff_changed` banner, the user sees the *banner* but reviews against *stale* code. *(Rust core)*
- **I9.** `PairingStore`'s derived `Default` ships `ttl = Duration::ZERO`. Currently unused but a footgun. `pairing.rs:11`. *(Rust core)*

### Security (defense-in-depth)

- **I10.** Non-constant-time bearer-token compare. `server/auth.rs:62`. Use `subtle::ConstantTimeEq`. *(Security + Rust core)*
- **I11.** `/v1/pair` has no rate limit / failure delay (currently functionally broken — fix at same time as C1). `pairing.rs:34`. *(Security)*
- **I12.** Git refs/SHAs forwarded to `git` CLI without leading-dash validation or `--` separators. `repo/worktree.rs:69`, `git/{diff,show,blame}.rs`. Exploit requires attacker-influenced `head_ref` or prompt-injected LLM tool call. *(Security)*

### Frontend / UX

- **I13.** `Shell` stores `height` but never applies it; drag-mouseup has no viewport clamp; window listeners can leak across teardown. *(Frontend)*
- **I14.** `SelectionLayer` swallows every Cmd-click document-wide, conflicting with OS "open in new tab". *(Frontend)*
- **I15.** Pairing deep-link path missing; spec calls it the default. *(Frontend)*
- **I16.** Per-session 🔇 presentation toggle missing. *(Frontend)*
- **I17.** `Options.tsx` lacks the spec'd `open_link` panel/tab settings. *(Frontend)*
- **I18.** Streaming conversation pane has no `aria-live`; export modal has no focus trap / Esc. *(Frontend)*

### Distribution

- **I19.** `libre-cr doctor` is missing two of four spec-required checks (port availability, code-daemon health probe). *(Distribution)*
- **I20.** Windows `send_term` is a no-op so graceful stop wastes the full 5 s deadline then hard-kills. *(Distribution)*
- **I21.** Log rotation is a TODO; logs grow unbounded. *(Distribution)*
- **I22.** README / CONTRIBUTING don't mention the wrapper, don't tell the user to put binaries on PATH, don't explain LLM-key configuration (especially while the config UI 404s). *(Distribution)*

### Test gaps

- **I23.** `SpawnedClient` reconnect/restart loop (`code_daemon/client.rs:297-440`) has zero coverage — `tests/spawned_daemon.rs` only exercises the happy path. *(Tests)*
- **I24.** Anthropic/OpenAI providers: only SSE-parser unit tests; no `mockito`-style HTTP integration; `validate()` is a no-op. This is the test gap that lets C4 ship undetected. *(Tests)*
- **I25.** `MockCodeDaemonClient` exposes 4 tools (`grep`/`read_file`/`find_references`/`git_log`) vs the real daemon's 19; schemas drift (mock uses `query`, real uses `pattern`). *(Tests)*

## Suggestions (worth doing post-v2)

Selected from the individual reports; not exhaustive.

- Centralize `repo_path` injection in `ToolContext`; remove repeated `shellexpand::tilde` boilerplate from every tool. *(Rust core)*
- Code daemon's `Mutex<Connection>` should be `tokio::sync::Mutex` or use a connection pool. *(Rust core)*
- Tighten the in-house JSON-schema validator to handle `enum`, `items`, nested objects. *(Rust core)*
- `MockProvider` / `MockCodeDaemonClient` fallback in production should be an explicit opt-in flag, not silent. *(Rust core)*
- Switch content script's React to `preact/compat` (drops ~125 kB; bundle is 196 kB today). *(Frontend)*
- `ServerFrame::ToolResult.result_preview` carries the full result — either rename or truncate. *(Rust core)*
- Shared `ScriptedClient` test double instead of two hand-rolled ones in `tools/router.rs` and `worktree.rs`. *(Tests)*
- Replace `tokio::time::sleep(80ms)` sync barrier in `ws_smoke.rs:114` with a real signal. *(Tests)*

## Confirmed good

Patterns the project gets right and shouldn't regress:

- **Architectural seams.** Three-category tool router; code-daemon ↔ review-daemon
  ownership split (`repo_path` vs `session_id` arguments); presentation
  call/result frames on the same WS as text streaming.
- **`RestartBudget` + supervisor sliding window.** Clean, well-tested, used on
  both the wrapper→review and review→code links.
- **`SpawnedClient` actor model.** Request multiplexing via id→oneshot map,
  exponential reconnect backoff with cap, fail-pending on EOF so callers never
  hang.
- **`safe_join` path-traversal defense.** Three-pass (absolute reject, lexical
  normalize, canonicalize-then-recheck) with full test coverage.
- **AES-GCM crypto** with random 12-byte nonce per encrypt, per-install 32-byte
  key file at 0600, round-trip + different-key tests in place.
- **WS `?token=` query-fallback** correctly gated on `Upgrade: websocket`,
  origin policy applied independently, header path unchanged.
- **`annotate_line` uses `textContent`** (no `innerHTML`) — XSS-safe on the
  one path where the LLM directly writes into the DOM.
- **`git worktree add --detach`** with no checkout hooks — worktree poisoning
  not a viable attack.
- **Severity ordering** in storage uses `derive(Ord)` with a documented
  precedence comment.
- **Migrations** are additive-only with a `_schema_version` table that refuses
  newer schemas.
- **CI** runs fmt/clippy/test on Linux+macOS+Windows; extension typecheck
  gated on `extension/package.json` so it doesn't break Phase 0.

## Suggested punch-list order

If you're prioritizing a v2.0 ship checklist:

1. **C1 + C2 + C3** (pairing flow + persistence). Single PR — they're the same
   coordination problem.
2. **C4** (provider streaming). One PR per provider; add the missing
   HTTP-integration tests (I24) in the same PR.
3. **C5 + C6 + C7** (frontend DOM safety + lifecycle).
4. **C8 + C9** (logs + config path).
5. **I3** (cancelled turn persistence) — small fix, easy regression risk.
6. **I10 + I11 + I12** (security hardening) — small focused PR.
7. Documentation pass for **I22** before any external announcement.

Everything else can ship as Phase 8 polish patches.

## Process notes

This certification was driven by 5 parallel review agents reading disjoint
domains, each producing a structured report under `REVIEW/`. No code was
modified during the review. The reports cite file paths and line numbers
throughout — re-running on a future commit will be straightforward.
