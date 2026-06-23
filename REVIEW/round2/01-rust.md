# Rust deep review — round 2

## Summary

The fix round delivered what it claimed: pairing is wired end-to-end, both provider stream parsers now buffer tool inputs correctly with good unit coverage, cancelled turns persist, token compare is constant-time, redemption is rate-limited, and git argv hardening (`validate_ref` + `--` separators) is real. 224 tests pass; clippy is clean. However, the config fix is only half-done — `POST /v1/config` persists to disk but **the running provider is never rebuilt**, so the documented config-ui flow still ends with the daemon answering from the mock provider until a manual restart, and `/v1/config/validate` validates the stale provider. That is the one finding I'd still call blocking. Beyond it: usage accounting is zero for both real providers, `pair_issue`'s TTL parameter is decorative, and six of the round-1 Important items are untouched.

## Critical

**C1 — Config changes persist but never apply; validate checks the stale provider.** `state.provider` is built once at startup (`crates/libre-cr-review/src/cli.rs:140`, `build_provider` at `cli.rs:218`) and stored as a fixed `Arc` in `AppState` (`server/state.rs:43`); `ws.rs:166` clones that same Arc per turn. `post_config` (`server/routes.rs:384-422`) mutates the `ConfigStore` and writes `review.toml`, but nothing rebuilds the provider, and the config-ui (`routes.rs:573`) says only "Changes are written to review.toml immediately" — no restart hint. Worse, `validate_config` (`routes.rs:440-443`) calls `state.provider.validate()`, i.e. the *old* provider, so a user who just entered an Anthropic key gets `{"ok":true}` from the mock. The round-1 C3 demo failure ("set key in config-ui, ask a question, get mock answers") is still reproducible; only the restart-loses-it half was fixed. Fix: rebuild the provider from the snapshot inside `post_config` (swap behind an `ArcSwap`/`RwLock<Arc<dyn Provider>>`, or resolve the provider from `ConfigStore` per turn in `ws.rs`), and make `validate_config` construct a provider from the *current* config.

## Important

**N1 — Usage tally is always zero against both real providers.** Anthropic: `input_tokens` arrives in `message_start` (`message.usage.input_tokens`) and `output_tokens`/`stop_reason` in `message_delta` — both ignored (`provider/anthropic.rs:309-311`); the parser reads them from `message_stop` (`anthropic.rs:312-329`), whose real payload is just `{"type":"message_stop"}`. OpenAI: the request never sets `stream_options: {"include_usage": true}` (`provider/openai_compat.rs:115-125`), and the synthesized `Done` hardcodes zeros (`openai_compat.rs:255-261, 319-327`). Net effect: `turns.usage_in/usage_out` (`agent/loop_.rs:214-235`, `loop_.rs:323-324`) is 0 for every production turn, and stop-reason-driven behavior (e.g. detecting `max_tokens` truncation) is impossible.

**N2 — `POST /v1/pair/issue` advertises a TTL it does not honor.** `pair_issue` clamps `ttl_seconds` to 30–3600 and computes `expires_at_epoch_ms` from it (`server/routes.rs:517-532`), but `PairingStore::issue()` takes no TTL and the store's TTL is fixed at 300 s (`pairing.rs:73,78-84`). A caller requesting 3600 gets an expiry timestamp an hour out for a code that dies in five minutes. Either plumb the TTL through or drop the parameter.

**N3 — Origin learned via `/v1/pair` is neither persisted nor reflected in CORS.** `pair` writes `extension_origin` into the in-memory config only (`routes.rs:497-500`) — unlike `post_config` it never calls `persist_config_atomic`, so pairing must be redone after every daemon restart. Separately, the CORS layer is built once at startup from `state.extension_origin` (`routes.rs:52-65`); the auth middleware reads the origin fresh (`auth.rs:38-44`), but browser preflights from the newly-paired origin are still evaluated against the stale (typically empty → allow-nothing) `CorsLayer` until restart. MV3 host-permission fetches may bypass CORS, but the two layers now intentionally disagree.

**N4 — `busy_sessions` can leak permanently.** The flag is inserted before `ws.on_upgrade` and removed only inside the upgrade callback (`server/ws.rs:34-47`). If the upgrade never completes (client drops between the 101 and hyper's upgrade — axum then skips the callback; there is no `on_failed_upgrade`), or if `handle_ws` panics, the session id stays in the set forever and every subsequent `/ask` for that session 409s until restart. Use a drop guard around the insert, plus `on_failed_upgrade` cleanup.

**N5 — Anthropic parser swallows `event: error` and drops state on early termination.** The API's mid-stream `error` SSE event falls into the `_ => {}` arm (`anthropic.rs:331`), after which the stream ends and `run_turn` reports the generic "provider stream ended without done" (`loop_.rs:232-234`) instead of the actual overloaded/invalid-request message. Also, unlike the OpenAI parser (which flushes buffered tools on `None`, `openai_compat.rs:218-233`), `anthropic_event_stream` ends silently at `sse.next().await?` (`anthropic.rs:192`), discarding any half-buffered tool block — harmless today only because the loop errors the turn anyway, but the asymmetry between the two state machines is a trap.

**N6 — MCP server is serial per connection.** `run_stdio`/`handle_socket_conn` await each `handle_line` before reading the next request (`libre-cr-code/src/mcp/server.rs:17-35, 78-95`). The review daemon's client actor multiplexes by id expecting concurrency, but a minutes-long `prepare_worktree` (whose `git fetch` is also a *blocking* `std::process::Command` on a runtime thread, `repo/worktree.rs:67-72`) head-of-line-blocks `ping`, health checks, and every other in-flight call. This makes round-1 I2/I4 worse than they looked individually.

## Suggestions

- Duplication from the fix round: `issue_pair_code_via_daemon` (`libre-cr-review/src/cli.rs:81-116`) and `commands/pair.rs:13-50` in the CLI are the same HTTP client twice; `Config::save` (`config.rs:188-196`) is now dead (only its own test calls it) and non-atomic next to `persist_config_atomic` (`routes.rs:426-438`) — delete one.
- `post_config` accepts any `provider.kind` string (`routes.rs:392-394`); `build_provider` rejects unknown kinds at startup (`cli.rs:245`), so a typo accepted today prevents the daemon from booting tomorrow. Validate on write.
- `tools_for_verb` ignores its verb argument entirely (`tools/router.rs:64-71`).
- OpenAI tool-call chunks lacking `index` collapse into entry 0 (`openai_compat.rs:285`); some Ollama/compat servers omit it — fall back to the call `id` as key.
- `PairingStore.failures` is never pruned (`pairing.rs:51`); bounded in practice on loopback, but add eviction alongside `prune`.
- `/config-ui` is skip-auth and reads `?token=` from a normal page navigation (`auth.rs:15-19`, `routes.rs:543`), which lands the bearer token in browser history — the WS-only query-token rationale from round 1 doesn't extend here.
- `review.toml` is written with umask-default perms (`routes.rs:435`); the spec's 0644-mode refusal check is still unimplemented (round-1 suggestion, open).
- `~/.local/state/libre-cr/log/libre-cr-code.log` is append-only with no rotation (`code_daemon/transport.rs:107-136`).

## Previously-flagged-still-open

- **I1 health hardcoded — OPEN.** `/v1/health` still returns `code_daemon: {connected: true, version: "mock"}` unconditionally (`routes.rs:445-451`) while `health_code_daemon` has the real hook one function below.
- **I2 serialized tool dispatch — OPEN.** `loop_.rs:268-295` still awaits each `dispatch` sequentially; aggravated by N6.
- **I4 blocking git in async — OPEN.** `std::process::Command` in async `prepare` (`libre-cr-code/src/repo/worktree.rs:67-98`); `RepoRegistry` still `std::sync::Mutex<Connection>` in async tools (`repo/registry.rs:8-11`).
- **I5 WS select structure — MOSTLY FIXED.** Double-spawn removed; `tokio::pin!` + `&mut reader_task` (`ws.rs:229-242`). Residual: the reader task is never aborted on the happy path (terminates via the close handshake — benign), and N4 above is the remaining lifecycle gap.
- **I6 ordinal race / non-transactional insert — OPEN.** `next_ordinal` and `insert_turn` take the lock separately (`storage/store.rs:160-230`); turn + FTS + traces are still separate executes with no transaction.
- **I7 unbounded flight map — OPEN.** (`repo/worktree.rs:14, 33-38`).
- **I8 no re-fetch on force-push — OPEN.** `prepare` short-circuits on `.git` existing (`repo/worktree.rs:61-64`); `create_session` computes `pr_diff_changed` but never re-prepares (`routes.rs:135-188`).
- **I9 PairingStore Default footgun — OPEN.** `#[derive(Clone, Default)]` still ships `ttl = 0` (`pairing.rs:46-53`); all production sites use `new()`.
- **Fixed and verified:** pairing wire-up (`/v1/pair/issue` + both CLIs + e2e `pair_issue_then_redeem`), config persistence-to-disk (atomic tmp+rename), constant-time compare (`auth.rs:90-92`, `subtle`), redemption rate limiting (per-IP window, 429 + Retry-After, well tested `pairing.rs:148-208`), git argv hardening (`util.rs:87-109` + `--` in diff/show/blame, validated at all six call sites), code-daemon stderr → log file (`transport.rs:83,110-136`), cancelled-turn persistence (`ws.rs:230-242` mirrors deltas via `PartialAnswer` and persists `TurnStatus::Cancelled`).

## Confirmed good

- Both stream state machines handle the hard cases correctly: per-index buffering of `input_json_delta` / `arguments` fragments, multiple concurrent tool calls (`HashMap<u64,…>` / `BTreeMap` for stable flush order), interleaved text + tool blocks, malformed SSE (error pushed, stream continues), duplicate-`Done` suppression (`done_sent`), and flush-on-EOF on the OpenAI side. Unit tests pin the exact round-1 regression (`anthropic.rs:377-422`, `openai_compat.rs:393-421`).
- `build_body` on the OpenAI side now splats `role:"tool"` messages per result (`openai_compat.rs:51-101`), fixing the round-1 `_tool_results` blob; the Anthropic side serializes `ContentBlock` to the exact wire shape (`provider/mod.rs:58-75`).
- `validate_ref` is tight (empty / leading `-` / whitespace / NUL) with both accept- and reject-shape tests (`util.rs:87-188`).
- Auth middleware: constant-time compare, WS-only query-token, origin allowlist read fresh per request (`auth.rs:21-47`).
- Router injection of `repo_path`/`repo_id` is schema-gated and well tested (`tools/router.rs:105-137, 233-367`); supervisor, `safe_join`, MCP framing, and the code-daemon client actor are unchanged from their round-1 "confirmed good" state.

## Coverage notes

Read end-to-end this round: both provider parsers + `provider/mod.rs`, `agent/loop_.rs`, `server/{ws,routes,auth,state}.rs`, `pairing.rs`, `config.rs`, review `cli.rs`, review `worktree.rs`, `tools/router.rs`, `storage/store.rs` (first 260 lines + round-1 knowledge), code-daemon `util.rs`, `repo/worktree.rs`, `git/{diff,show,blame,log}.rs`, `mcp/server.rs`, CLI `supervisor.rs` and `commands/pair.rs`, transport stderr path. Ran `cargo clippy --workspace --all-targets` (clean) and the full test suite (all green). Skimmed only: e2e suites (entry points + pairing tests), `export.rs`, `presentation.rs`, `registry.rs` (mutex type check). Not exercised: real Anthropic/OpenAI endpoints — N1/N5 are derived from the documented SSE event shapes, not live traffic.
