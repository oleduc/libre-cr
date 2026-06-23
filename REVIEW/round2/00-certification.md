# Libre CR — Round-2 Certification

Second full review after the round-1 fix cycle (16 fixes), the E2E restructure
(`crates/libre-cr-e2e/`, `extension/e2e/`, `extension/e2e-browser/`), and the
Playwright browser suite. Four independent reviewers:

- [`01-rust.md`](01-rust.md) — fresh deep review of all five crates
- [`02-extension.md`](02-extension.md) — fresh extension review + fix checks
- [`03-architecture.md`](03-architecture.md) — architecture audit (new this round)
- [`04-fix-verification.md`](04-fix-verification.md) — adversarial verification of all 16 claimed fixes

## Verdict

**Architecture: sound-with-erosion-risks. Implementation: one Critical
remaining, ship-blocking for the configuration UX; everything else is
Important-tier.** The round-1 fix cycle was real: 14 of 16 fixes verified
outright, 2 partial, 0 fake, 0 regressions introduced. All gates green at
review time: 224 Rust + 60 unit + 5 node-E2E + 7 browser tests.

The product's demo path now works end-to-end **except** that changing the
provider config via the new config UI doesn't take effect until a manual
daemon restart — which is the exact user journey the config UI was built for.

## Critical (1)

**RC1 — Config changes don't reach the running provider.**
`POST /v1/config` persists to disk (round-1 C3 fix is real), but
`state.provider` is an `Arc` built once at startup
(`crates/libre-cr-review/src/cli.rs:140`); nothing rebuilds it on config
change, and `POST /v1/config/validate` validates the *stale* provider
(`routes.rs:440`). User flow: open config-ui → enter Anthropic key → save →
validate says OK → ask a question → still get mock answers. Fix: store the
provider behind `ArcSwap`/`RwLock` and rebuild on accepted config mutation, or
at minimum return "restart required" from the route and surface it in the UI.
*(01-rust C1; corroborated by 04-fix-verification.)*

## Important — new this round

### Provider layer
- **N1 — Token usage always reports zero from real providers.** Anthropic
  parser reads `usage`/`stop_reason` from `message_stop` (the real API sends
  them in `message_start`/`message_delta`); OpenAI never sets
  `stream_options: {include_usage: true}`. Loop correctness unaffected
  (turn-end keys on empty `tool_uses`), but the spec'd `done.usage` frame and
  the per-turn `usage_in/usage_out` columns are garbage. *(01-rust N1)*
- **N5 — Stream-parser asymmetry.** Anthropic parser ignores `event: error`
  frames and silently drops buffered tool state on early EOF; the OpenAI
  parser flushes. Same-shaped state machines should fail the same way.
  *(01-rust N5)*

### Server lifecycle
- **N4 — `busy_sessions` leaks on failed WS upgrade or handler panic** →
  that session 409s forever (restart to clear). The guard should be an RAII
  drop-guard, not manual insert/remove. Also flagged by the architecture
  audit as mislocated: it lives in the WS transport so the planned external
  MCP `ask_about_pr` path won't be covered by it. *(01-rust N4; 03-arch)*
- **N3 — Origin learned via `/v1/pair` isn't persisted and CORS is built
  once at startup.** Same bug class as round-1 C3, different route: pairing
  succeeds, extension origin is recorded in memory, lost on restart; and the
  in-process CORS layer never picks it up even before restart. *(01-rust N3)*
- **`post_config` returns `ok: true` even when the disk write fails.**
  *(04-fix-verification)*
- **N2 — `pair_issue` echoes a `ttl_seconds` it never applies** (store TTL
  fixed at 300 s). Cosmetic lie in the API. *(01-rust N2; 04-fix)*
- **I11 partial — pairing rate-limit `failures` map only pruned on
  successful redeem**; unbounded growth under rotating-IP failures.
  *(04-fix-verification)*

### Extension
- **E1 — The 🔇 mute toggle is a placebo.** It persists state and sends
  `mute_presentations` in `AskInit`, but the Rust `AskInit` struct has no such
  field (`ws_frames.rs:15-21`) and the extension attaches the presentation
  manager unconditionally (`QaPanel.tsx:170`). Presentations render while
  muted. Fix needs both sides: field on the Rust struct + tool-router gate,
  or at minimum a local gate in the presentation handler. *(02-ext; 04-fix)*
- **E2 — Auto-collapse of older turns is broken.** `ConversationTurn.tsx:71`
  seeds collapse state from props via `useState` once and never re-syncs, so
  the panel's collapse-older logic is a no-op. Untested. *(02-ext)*
- **C7 residual — a synchronous `new WebSocket()` constructor throw still
  leaves `inflight = true` forever** (`ws.ts:93-95`). Narrow but real; wrap
  the constructor in try/catch inside `open()`. *(04-fix-verification)*

### Test infrastructure
- **E2E suites vacuously pass when binaries can't be built** (`if (!daemon)
  return` / skip-on-build-failure). Right behavior locally; in CI it means a
  broken Rust build can show green E2E. Add an env flag (e.g.
  `LIBRE_CR_E2E_REQUIRED=1` in CI) that turns skip into failure.
  *(04-fix-verification)*

## Architecture audit findings

Verdict: **sound-with-erosion-risks**. The load-bearing invariants verified
intact: ownership boundary (review daemon touches repos only via MCP; code
daemon has zero PR/LLM knowledge), crate graph matches spec (`common` is the
sole shared dep), supervision topology has no double-supervision and orphaned
code daemons self-terminate on stdin EOF.

Ranked erosion risks (full detail in [`03-architecture.md`](03-architecture.md)):

1. **Untyped HTTP responses.** Route bodies are `json!` literals; the response
   types exist only in the extension's `frames.ts`. This is the #1 drift
   surface between Rust and TS. Prevention: define response structs in
   `libre-cr-common`, derive `Serialize`, and mirror once.
2. **Error vocabulary mismatch is already live.** Code daemon emits
   `"invalid_input"`, which exists in neither `common::ErrorCategory` nor the
   TS union. The envelope contract is string-convention only.
3. **`PROTOCOL_VERSION` is dead code** — never sent, never checked. The
   spec's minor-version wire-compat promise has no enforcement mechanism.
4. **GitHub is hardwired in the wrong layer** — `parse_pr_url` lives in the
   review daemon's *store* (`store.rs:32-50`) and the extension has no
   platform-adapter indirection. GitLab/Bitbucket (a spec'd "Later" item) is
   currently 3-layer surgery. Cheapest prep: extract a `PlatformRef` type now.
5. **Migration asymmetry** — review crate has versioned forward-refusing
   migrations; the code daemon's repo registry has none.
6. **Failure-matrix gaps** — wrapper SIGKILL leaves a live unsupervised
   review daemon that `start` then mislabels "already running";
   `worktree_path` cached on session rows has no invalidation against future
   LRU eviction; production `serve` silently falls back to
   `MockCodeDaemonClient` when the real spawn fails (round-1 suggestion, now
   architecture-tier: a misconfigured install gets fake answers with no
   warning).

Growth-seam assessment: **Phase C (LSP) additive** (the `Tool` trait +
per-call backend selection seam is real); **plugin verbs minor surgery**;
**GitLab the weakest seam** (see #4).

## Fix-verification summary

| Verdict | Count | Items |
|---|---|---|
| VERIFIED | 14 | C1, C2, C3*, C4*, C5, C6, C7*, C8, C9, I3, I10, I12, I15, I18 |
| PARTIAL | 2 | I11 (map leak), I16 (placebo mute) |
| NOT FIXED | 0 | — |

\* with the adjacent gaps noted above (C3: sibling route unpersisted; C4:
usage fields wrong; C7: constructor-throw residual).

No regressions from the fix round. The restructured E2E suites genuinely
execute under `cargo test --workspace` (21 + 21 + 3) — verified by timing, not
just exit code.

## Round-1 items still open (unclaimed, confirmed)

I1 (health hardcodes code-daemon state), I2 (serialized tool dispatch — spec
says parallel), I4 (blocking `git` subprocess on tokio workers — compounds
with N6: the MCP server is serial per connection, so a minutes-long fetch
head-of-line-blocks everything), I6 (ordinal race on concurrent note POSTs),
I7 (unbounded worktree flight map), I8 (no re-fetch on force-pushed PR refs —
user reviews stale code while the banner says the diff changed), I9
(`PairingStore::default()` TTL=0 footgun), I13 (Shell height/clamp/listener
leaks), I14 (document-wide Cmd-click hijack), I17 (open_link settings absent
from Options), I19–I25 (doctor gaps, Windows stop, log rotation, README,
SpawnedClient restart-loop coverage, provider HTTP-level tests, mock schema
drift).

## Suggested priority order

1. **RC1** — provider hot-reload (or explicit restart-required UX). The only
   remaining demo-stopper.
2. **N3 + N4** — origin persistence + busy-session RAII guard. Both are
   "works in the demo, breaks the second session" bugs.
3. **E1 + E2** — make the mute toggle real (both sides) and fix turn collapse.
4. **N1 + N5** — provider usage + parser symmetry, together with the
   still-open I24 (HTTP-level provider tests) so this layer finally has
   integration coverage.
5. **Architecture preventive work** — typed responses in `libre-cr-common`,
   unify the error vocabulary, send/check `PROTOCOL_VERSION`. Cheap now,
   expensive after a GitLab adapter or plugin-verb work begins.
6. **I8** — force-push re-fetch; correctness issue for the core workflow.
7. Everything else as Phase-8 polish.

## Process notes

Four parallel read-only reviewers; the fix-verification pass independently
re-ran every gate (quoted numbers, not inherited claims) and adversarially
probed each claimed fix. No code was modified during this round.
