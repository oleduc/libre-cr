# Fix verification — libre-cr round 2

## Summary

**14 of 16 verified fixed; 2 partial (I11, I16); 0 not fixed.** All gates green, run locally on this commit:

- `cargo test --workspace`: **224 passed, 0 failed** (cli 22+3, code 35+5, common 5, e2e 21+21+3, review 87+1+9+6+3+3 + cli bin/doc 0s)
- `extension: pnpm test`: **60 passed** (18 files)
- `extension: pnpm test:e2e`: **5 passed** (daemon-roundtrip, real spawned `libre-cr-review`)
- `extension: pnpm test:browser`: **7 passed** (Playwright, chromium installed — ran, not skipped; 11.4s)

## Verification table

| Finding | Claimed fix | Verdict | Evidence |
|---|---|---|---|
| C1 pairing E2E | `POST /v1/pair/issue`, CLIs hit running daemon | **VERIFIED** | `routes.rs:83,517`; auth NOT skipped (`auth.rs:19` lists only `/v1/health`, `/v1/pair`, `/config-ui`); CLIs: `libre-cr-cli/src/commands/pair.rs:26`, `libre-cr-review/src/cli.rs:96`; single-use via `HashMap::remove` (`pairing.rs:94`). Tests: `http_api.rs:190` (asserts unauthenticated issue → 401), e2e `http_consumer.rs:228`, Playwright `pairing.spec.ts` |
| C2 config 404 | `GET /config-ui` HTML + wrapper URL | **VERIFIED** | `routes.rs:84,543-551`; wrapper opens `/config-ui?token=` (`commands/config.rs:23`). Test `http_api.rs:114 config_ui_serves_html` |
| C3 config persistence | Persist on POST, atomic rename | **VERIFIED** | `routes.rs:414-437`; tmp file is `<path>.tmp` in the **same directory** → same filesystem → rename is atomic. Tests: `http_api.rs:135 config_post_persists_to_disk`; e2e `http_consumer.rs:322 config_post_persists_across_restart` (actual daemon restart) |
| C4 provider tool-input buffering | Buffer per index, emit once | **VERIFIED** | Anthropic: `anthropic.rs:243-307` (HashMap by block index, flush at `content_block_stop`); `content_block_stop` for a never-started index (incl. text blocks) is a safe no-op (`remove`→`None`, :287). OpenAI: `openai_compat.rs:283-308` BTreeMap by `tool_calls[].index` — two interleaved calls buffer independently; flush on `finish_reason`/`[DONE]`/EOF with `done_sent` guard. Tests: `buffers_tool_input_across_deltas_and_emits_once` (anthropic.rs:377), `buffers_tool_arguments_across_chunks_and_emits_once` (openai_compat.rs:393), `empty_tool_input_defaults_to_object` |
| C5 popup innerHTML | Drop `dangerouslySetInnerHTML` | **VERIFIED** | Zero hits in `entrypoints/`+`components/`; `renderSnippet` returns React nodes (`Popup.tsx:182`). Test `popup-snippet.test.tsx` (3) |
| C6 orphaned effects | clearAll/detachAll on unmount | **VERIFIED** | `QaPanel.tsx:43-53` effect cleanup calls `m.clearAll(); m.detachAll()`. Test `qa-panel-cleanup.test.tsx` asserts `data-libre-cr-effect-id` rows removed from the page DOM on unmount |
| C7 `inflight` lock | Clear in error/close handlers | **VERIFIED** (residual, below) | `ws.ts:100-109` — `settle()` clears `inflight` and is invoked from `onerror` (:133), `onclose` (:136), done/error frames (:127-131); `close()` also (:194). Covered by e2e `ws_session_streams_frames` + unit ws tests |
| C8 code-daemon stderr | Pipe to `libre-cr-code.log` | **VERIFIED** | `transport.rs:83` `stderr(stderr_log_stdio())` → `~/.local/state/libre-cr/log/libre-cr-code.log` ($XDG_STATE_HOME honored, :140-148); `libre-cr logs` lists it (`commands/logs.rs:11`, `paths.rs:73`). No automated test of the log path (falls back to `Stdio::null()` on open failure) |
| C9 code.toml namespace | `~/.config/libre-cr/code.toml` | **VERIFIED** | `libre-cr-code/src/config.rs:119`; one-time migration from legacy `libre-cr-code/config.toml` (:147-161); unit test asserts new namespace and rejects old (:221-228) |
| I3 cancelled persistence | Use `persist_cancelled` | **VERIFIED** | `server/ws.rs:237` calls it; `#[allow(dead_code)]` gone (`loop_.rs:333`). Tests: `ws_smoke.rs:80 ws_cancelled_turn_is_persisted_with_partial_answer`; e2e `http_consumer.rs:867 ws_ask_cancellation_persists_partial` |
| I10 constant-time compare | `subtle::ConstantTimeEq` | **VERIFIED** | `auth.rs:9,90-92 tokens_eq` used on both header and WS-query paths (:68,:76). Unit tests `auth.rs:134+` |
| I11 pair rate limit | Per-IP failure window, 429 | **PARTIAL** | `pairing.rs:99-127` (5 failures / 60 s / per IP, reset on success); `routes.rs:488-494` returns 429 + `Retry-After: 60`. Tests: `pairing.rs:167-207` (4), `http_api.rs:241`, e2e `http_consumer.rs:262`. **Hole: the `failures` map is never pruned** — see below |
| I12 ref validation | `validate_ref` at all cited sites | **VERIFIED** | `worktree.rs:47`, `diff.rs:27-28`, `show.rs:17`, `blame.rs:33`, `log.rs:27` (log is gix in-process — no argv at all); `--` separators at `blame.rs:36`, `diff.rs:36`, `show.rs:50`. Test `util.rs:180 validate_ref_rejects_argv_injection` |
| I15 pairing deep-link | `#pair?endpoint=&code=&auto=1` | **VERIFIED** | `Options.tsx:14-25` parser, auto-pair at :72. Tests `options-deeplink.test.tsx` (3) + Playwright `pairing.spec.ts:26` (auto-pair via deep link, real daemon) |
| I16 per-session 🔇 | Mute toggle | **PARTIAL** | Toggle exists, persists per session, correct ARIA (`QaPanel.tsx:87-111,314-323`). **But it doesn't actually mute anything** — see below |
| I18 aria-live / focus trap | Conversation + export modal | **VERIFIED** | `QaPanel.tsx:387 aria-live="polite"`; `ExportModal.tsx:71-100` Esc-to-close, Tab trap, initial focus. Tests `qa-panel-a11y-mute.test.tsx` (4), `export-modal.test.tsx` (3) |

## Holes found in fixes

1. **I16 mute is cosmetic — presentations are not suppressed.** The client sends `mute_presentations` in `AskInit` (`QaPanel.tsx:206`), but the daemon's `AskInit` struct has no such field (`libre-cr-common/src/ws_frames.rs:15-21` — serde silently drops it), the agent loop never reads it, and the client attaches the presentation manager unconditionally (`QaPanel.tsx:170`) with no muted gate in `utils/presentation/index.ts`. Repro: toggle 🔇, ask a question whose turn issues `highlight_lines` — the highlight still renders. The covering test (`qa-panel-a11y-mute.test.tsx:47`) only asserts the *preference* persists, not that anything is muted.
2. **I11 per-IP failure map leaks.** `PairingStore.failures` (`pairing.rs:51`) gains an entry per unique failing source IP and only ever removes one on a *successful* redeem from that IP (`pairing.rs:112`). `FailureWindow::record` resets the counter after 60 s but never the map entry. On a daemon exposed beyond localhost, a spoofed/rotating-IP sweep grows it without bound. Fix sketch: prune entries whose `first_failure` is older than the window inside `redeem_from`, mirroring `prune()` on the code map.
3. **C7 residual: constructor throw still locks the session.** `open()` sets `inflight = true` (`ws.ts:89`) before `new WebSocket(url)` (:95). A synchronous constructor throw (invalid URL) rejects the returned promise *without* running `settle()`, leaving `inflight` true forever — the exact "early throw before WS opens" class from round 1, on its narrowest path.
4. **C1 cosmetic: `ttl_seconds` is accepted but ignored.** `pair_issue` clamps the requested TTL and reports `expires_at_epoch_ms` from it (`routes.rs:521-527`), but `PairingStore::issue` always uses the fixed 300 s store TTL (`pairing.rs:73`). A caller requesting `ttl_seconds: 3600` gets a code that actually dies at 5 min while the response claims 60 min.
5. **C3 adjacent: pairing-time `extension_origin` is never persisted.** `pair` mutates in-memory config only (`routes.rs:497-500`) — unlike `post_config`, no `persist_config_atomic` call. After a daemon restart the origin allowlist learned at pairing is lost (auth falls back to allow-any-origin given a valid token, `auth.rs:112`). Same in-memory-only bug class C3 was about, on a different route.
6. **C3 adjacent: persist failure is silent to the caller.** `post_config` returns `{"ok": true}` even when the disk write fails (`routes.rs:414-420`, warn-log only) — the user's API key will vanish on restart with no signal.
7. **C4 adjacent: usage/stop_reason fidelity.** The real Anthropic API carries `usage`/`stop_reason` on `message_delta`, which the parser ignores (`anthropic.rs:309-311`); `message_stop` (where it reads them, :312-329) doesn't carry them, so token tallies report 0. OpenAI never requests `stream_options.include_usage`, so likewise 0. Loop *correctness* is unaffected — continuation keys on `tool_uses.is_empty()` (`loop_.rs:237`), not `stop_reason`. I24 (HTTP-level provider integration tests) remains open, so this class is still only unit-tested.

## Regressions introduced by the fix round

None found. `serve()` is the only `build_router` consumer and now uses `into_make_service_with_connect_info` (`routes.rs:101-107`), so the new `ConnectInfo` extractor in `pair` can't 500 elsewhere. Persisted `review.toml` contains only `api_key_enc` (AES-GCM ciphertext, `routes.rs:407-410`) — no plaintext secret lands on disk. The bearer token still lives in its separate 0600 file. fmt/clippy-sensitive areas compile clean in the workspace run.

## E2E restructure verification

- `crates/libre-cr-e2e` is a workspace member (`Cargo.toml:8`) and **executes** under `cargo test --workspace`: `http_consumer` **21 passed** (17.9 s), `mcp_consumer` **21 passed** (1.8 s), `spawned_smoke` **3 passed** (1.3 s). Coverage is real consumer-grade: pair issue/redeem, rate-limit 429, config persistence across an actual restart, WS cancel persistence, 409 on concurrent ask, query-token gating.
- Caveat: the suite self-builds binaries via `ensure_built()`/`locate_bin` (`tests/common/spawned_daemon.rs:60-79`) and tests **return early (vacuous pass)** if binaries can't be built — same pattern in `extension/e2e` (`daemon-roundtrip.test.ts:204-221`, `if (!daemon) return`). Locally they demonstrably ran (wall times, real HTTP); CI should assert non-skip or fail loudly.
- `extension/e2e`: 5/5 passed against a real spawned `libre-cr-review`. `extension/e2e-browser`: 7/7 Playwright tests passed (content-script attach, 3 pairing incl. deep-link, presentation round-trip, 2 Q&A panel) — browsers were installed, suite not skipped.

## Remaining open items from round 1

- **I1** — still open: `/v1/health` still hardcodes `code_daemon: {connected: true, version: "mock"}` (`routes.rs:445-451`); partially mitigated by new `/v1/health/code-daemon` which does use `health_hook` (:453-469).
- **I2** — still open: tool calls dispatched serially in a `for` loop (`loop_.rs:269-294`).
- **I4** — still open: blocking `std::process::Command::output()` in async `prepare` (`worktree.rs:67,92`).
- **I5** — not re-verified in depth; WS select structure unchanged in spirit (`ws.rs`).
- **I6** — still open: `next_ordinal`/`insert_turn` under separate locks (`storage/store.rs`).
- **I7** — still open: single-flight `lock_for` map grows per unique key forever (`worktree.rs:33-38`).
- **I8** — still open: `.git`-exists short-circuit skips re-fetch of force-pushed refs (`worktree.rs:60-64`).
- **I9** — still open: `#[derive(Default)]` on `PairingStore` still yields `ttl == 0` (`pairing.rs:46-53`).
- **I13** — partially moved: `Shell.tsx` now tracks `height` (:46) but clamping/leak claims not re-verified; treat as open.
- **I14** — likely improved un-claimed: cmd-click handler now early-returns when `hitTestLine` misses (`SelectionLayer.tsx:17-20`); not formally verified.
- **I17** — still open: no `open_link` panel/tab settings in `Options.tsx`.
- **I19** — partially improved: `doctor.rs:59-66` now has structured checks incl. endpoint reachability; port-availability and code-daemon probes not confirmed complete.
- **I20** — still open: Windows `send_term` path unchanged (`supervisor.rs:201`).
- **I21** — still open: no log rotation anywhere in `libre-cr-cli`.
- **I22** — not re-audited; README/CONTRIBUTING claims unverified.
- **I23** — partially improved: e2e `spawned_smoke.rs` exercises a real spawned client (list/read/grep) but still no reconnect/restart-loop coverage.
- **I24** — still open: providers have unit-level SSE parser tests only; no mockito/wiremock HTTP integration; `validate()` still only checks key non-emptiness (`anthropic.rs:138-143`).
- **I25** — still open: `MockCodeDaemonClient` still exposes 4 tools (`tools/code_daemon.rs:29-66`) vs the real daemon's catalog.
