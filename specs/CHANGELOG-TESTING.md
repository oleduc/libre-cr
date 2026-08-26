# Changelog — Testing & Certification

This file is a ledger of changes that were **driven by the certification rounds
and manual testing**, as distinct from the original design in the other
`specs/*.md` files. The specs describe the system *as built*; this file records
*why* the built system diverges from what was first written, with traceable
finding IDs.

Two formal certification rounds preceded manual testing:

- **Round 1** — `REVIEW/00-certification.md` (+ `REVIEW/01..05-*.md`). Findings
  `C1`–`C9` (Critical), `I1`–`I25` (Important), plus suggestions.
- **Round 2** — `REVIEW/round2/00-certification.md` (+ `REVIEW/round2/01..04-*.md`).
  Re-review after the round-1 fix cycle; `RC1` (Critical) and `N1`–`N5` (new
  Important), plus partials on round-1 items and an adversarial
  fix-verification pass (`REVIEW/round2/04-fix-verification.md`).

Manual testing after round 2 added two capabilities that no certification
round had reviewed: live model lists and ambient API-key detection.

Finding-ID conventions: `Cn`/`In` = round-1 Critical/Important; `RC1`/`Nn` =
round-2 Critical/Important; `manual testing — <observation>` for the
post-certification work.

---

## Round-1 certification fixes

Applied after `REVIEW/00-certification.md`.

- **Pairing wired through the running daemon.** CLIs issued codes locally; the
  daemon's store never saw them, so `POST /v1/pair` always 401'd. Added
  token-authed `POST /v1/pair/issue`; both CLIs hit it.
  *Trigger: C1. Specs: 04 § Pairing, § Configuration UI; 08 § Wrapper CLI Surface.*
- **`libre-cr config` opens a real page.** Added a daemon-served `GET /config-ui`
  HTML page (404'd before); wrapper opens it.
  *Trigger: C2. Specs: 04 § Configuration UI; 02 transports; 08.*
- **`POST /v1/config` persists to disk.** Provider/key edits were in-memory only
  and lost on restart. Now atomically written to `review.toml`.
  *Trigger: C3. Specs: 04 § Config.*
- **Providers buffer streamed tool inputs.** Anthropic ignored `input_json_delta`;
  OpenAI emitted a `ToolUse` per arguments-chunk. Now fragments accumulate per
  block/call id and emit one well-formed `ToolUse`.
  *Trigger: C4. Specs: 04 § LLM Provider Layer.*
- **Popup drops `dangerouslySetInnerHTML`** for search snippets (C5);
  **presentation effects cleared on Turbo nav** via `clearAll()` on unmount (C6);
  **`AskSession.inflight` cleared in WS error/close handlers**, not only
  `close()` (C7). *Specs: 05.*
- **Code-daemon stderr captured** to `~/.local/state/libre-cr/log/libre-cr-code.log`
  and surfaced by `libre-cr logs`. *Trigger: C8. Specs: 08 § Supervision Model.*
- **Code-daemon config moved** to `~/.config/libre-cr/code.toml` (was
  `~/.config/libre-cr-code/config.toml`), with one-time legacy migration.
  *Trigger: C9. Specs: 03 § Configuration; 08 § Configuration Layout.*
- **Cancelled turns persisted** (`persist_cancelled` was dead code).
  *Trigger: I3. Specs: 04 § Agent Loop.*
- **Force-push re-fetch** — `prepare_worktree` re-fetches and `reset --hard`s a
  diverged worktree instead of short-circuiting on `.git` existence.
  *Trigger: I8. Specs: 03 § Worktree management.*
- **Security hardening:** constant-time bearer compare (I10); pairing rate-limit
  + per-code TTL (I11, also see RC-round N2); git ref/SHA dash-injection guards
  (I12). *Specs: 04 § Pairing; 03 (unchanged surface).*
- **Extension UX:** Shell height/clamp/listener leaks (I13); document-wide
  Cmd-click hijack scoped (I14); pairing deep-link added as default path (I15);
  per-session 🔇 mute toggle added (I16 — completed in round 2, see below);
  `open_link` settings added to Options (I17); `aria-live` + export focus trap
  (I18). *Specs: 05.*
- **Distribution:** `libre-cr doctor` port + code-daemon-health checks (I19);
  README/CONTRIBUTING wrapper + PATH + key-config docs (I22). *Specs: 08.*

---

## Round-2 certification fixes

Applied after `REVIEW/round2/00-certification.md`. The round-1 fix cycle was
verified real (14/16 outright, 2 partial, 0 fake); these address the remaining
Critical and the new Important findings.

- **Provider hot-reload — the round-2 ship-blocker.** `POST /v1/config` persisted
  but the running `state.provider` was built once at startup and never rebuilt,
  and `/v1/config/validate` validated the *stale* provider — so a freshly-entered
  Anthropic key still returned mock answers. The provider is now held behind a
  swappable cell; the route builds + swaps it on every accepted mutation (and is
  transactional: prove-construct → persist → commit → swap), and `validate`
  builds the candidate/stored provider fresh.
  *Trigger: RC1. Specs: 04 § Config.*
- **Real token-usage tallies.** Anthropic parser now reads `input_tokens` from
  `message_start` and `output_tokens`/`stop_reason` from `message_delta` (not
  `message_stop`); OpenAI requests `stream_options.include_usage`. The `done`
  frame and `usage_in`/`usage_out` columns are accurate.
  *Trigger: N1. Specs: 04 § Agent Loop.*
- **Stream-parser symmetry.** Anthropic parser now turns `event: error` frames
  into `StreamEvent::Error` and flushes buffered tool state on early EOF, matching
  the OpenAI parser. *Trigger: N5. Specs: 04 § LLM Provider Layer.*
- **Busy-session RAII guard.** The single-in-flight claim is now a drop-guard, so
  a failed WS upgrade or handler panic releases the session instead of wedging it
  at `409` until restart. *Trigger: N4. Specs: 04 § Concurrency and Cancellation.*
- **Persisted `extension_origin` + dynamic CORS.** The origin learned on
  `/v1/pair` is written to `review.toml` and applied to a CORS layer that reads
  the live allowlist per request — effective immediately and across restarts.
  *Trigger: N3. Specs: 02 transports; 04 § Pairing, § Configuration UI.*
- **`post_config` returns the real outcome** — a failed disk write is a `500`
  with nothing applied, not a silent `{ok:true}`.
  *Trigger: round-2 fix-verification. Specs: 04 § Config.*
- **Pairing per-code TTL actually applied.** `POST /v1/pair/issue` now applies the
  requested (clamped) TTL to the stored code instead of only echoing it.
  *Trigger: N2. Specs: 04 § Pairing.*
- **Pairing rate-limit map pruned** beyond just on successful redeem (bounded
  under rotating-IP failures). *Trigger: I11 partial. Specs: 04 § Pairing.*
- **Mute toggle made real (both sides).** `AskInit` gained a `mute_presentations`
  field; a muted turn does not register presentation tools at all, and the
  extension also gates locally (`presentation_muted`). Previously a placebo.
  *Trigger: E1 / I16. Specs: 04 § Ask/streaming Q&A; 05 § Presentation Handler.*
- **Turn auto-collapse fixed.** Collapse is now a controlled prop owned by the
  panel rather than seeded once from local state. *Trigger: E2. Specs: 05 § Q&A Panel.*
- **`new WebSocket()` constructor throw** wrapped so `inflight` can't stick.
  *Trigger: C7 residual. Specs: 05 (transport).*

### Round-2 architecture-audit fixes

- **Typed HTTP responses in `libre-cr-common`.** Route bodies were `json!`
  literals; response shapes now live as `Serialize`/`Deserialize` structs in
  `http_api.rs`, mirrored by the extension's `frames.ts`.
  *Trigger: arch erosion #1. Specs: 02 § Typed HTTP wire contract.*
- **Unified error vocabulary.** Code daemon's `invalid_input` renamed to
  `validation_failed`, matching `libre_cr_common::ErrorCategory`.
  *Trigger: arch erosion #2. Specs: 03 § Error Model; 04 error table.*
- **`PROTOCOL_VERSION` is now live** — sent in `GET /v1/health` and soft-checked by
  the extension. *Trigger: arch erosion #3 / I1-adjacent. Specs: 02, 04 § Health, 05.*
- **Real code-daemon health in `/v1/health`** via the wrapper's health hook
  (mock fallback only for in-process tests). *Trigger: I1. Specs: 04 § Health.*
- **Parallel tool dispatch** in the agent loop via `join_all`, result blocks
  reassembled in model order. *Trigger: I2. Specs: 04 § Agent Loop.*
- **Async git subprocesses** (`tokio::process`) so a slow `fetch` can't block a
  worker thread. *Trigger: I4. Specs: 03 § Concurrency.*
- **Concurrent, bounded MCP request dispatch** so a slow tool call doesn't
  head-of-line-block a connection. *Trigger: I4/N6-adjacent. Specs: 03 § Concurrency.*
- **Transactional ordinal assignment** (`insert_turn_auto_ordinal`).
  *Trigger: I6. Specs: 04 § Agent Loop.*
- **Single-flight map pruned** (`Weak` entries, pruned on acquire).
  *Trigger: I7. Specs: 03 § Concurrency.*
- **Repo-registry schema versioning** added (forward-refusing migrations).
  *Trigger: arch erosion #5. Specs: 03 § Repo registry.*

---

## Manual-testing changes

Capabilities added during manual testing / certification of the demo path,
beyond what either certification round reviewed.

- **Popup "Configure daemon" token fix.** The link now appends `?token=` so the
  config UI can authenticate (it 401'd on every JSON call otherwise).
  *Trigger: manual testing — config UI unauthorized when opened from the popup.
  Specs: 05 § Popup; 04 § Configuration UI.*
- **Live model lists.** Added `POST /v1/provider/models` (lists a *candidate*
  provider's models without saving) and a `list_models` provider method;
  Anthropic implements it via `GET /v1/models`. The config UI gained a "Fetch
  models" dropdown. *Trigger: manual testing — users had to hand-type model ids.
  Specs: 01 § LLM provider and credentials; 04 § Config, § LLM Provider Layer,
  § Configuration UI.*
- **Ambient API-key detection.** Added `GET /v1/provider/detected` and an env-var
  fallback in provider construction: a saved key wins, else `ANTHROPIC_API_KEY` /
  `OPENAI_API_KEY` is used. The config UI shows a "detected — leave blank" hint.
  *Trigger: manual testing — users with the env var set still had to paste a key.
  Specs: 01; 02 state table; 04 § Config, § LLM Provider Layer; 08 § Required Host Software.*
- **Claude Code OAuth login — prototyped, then removed.** A `claude_code`
  provider that reused a local Claude Code login was built during manual testing
  and scrapped before commit: Anthropic's terms restrict Claude Code /
  subscription OAuth tokens to Claude Code itself, so a third-party daemon may
  not use them. Only `anthropic` (API key) and `openai_compat` remain.
  *Trigger: manual testing / ToS review.*
- **macOS config-path fix.** Both daemons resolved their default config path via
  `dirs::config_dir()` — on macOS that is `~/Library/Application Support`, so the
  daemons silently ignored the `review.toml`/`code.toml` under `~/.config/libre-cr/`
  that the wrapper, docs, and the daemons' own token/endpoint files use. Now
  resolved as `$XDG_CONFIG_HOME` → `~/.config`, with a one-time migration copying
  a file stranded at the Application Support location. All 285 tests had missed
  this because the E2E harness passes `--config` explicitly.
  *Trigger: live browser-driven manual testing — daemon served defaults instead of
  the written config. Specs: 04 § Configuration; 08 § Configuration Layout.*
- **Partial config sections parse with defaults.** A minimal `[provider]` block
  (exactly what the docs show) crashed the daemon with `missing field max_tokens`;
  all config section structs now carry container-level `#[serde(default)]`.
  *Trigger: same session — supervisor restart-budget tripped on parse failure.
  Specs: 04 § Configuration ("config files are forward-compatible").*
- **`libre-cr start` stale-endpoint banner.** The endpoint watcher returned a
  previous run's endpoint file instantly; the file is now removed before spawning
  so the banner reports the fresh port.
  *Trigger: same session — banner showed a dead port. Specs: 08 § First-Run Flow.*

---

## Still open / deferred

Known-imperfect items the certifications flagged that were **intentionally not
fixed** (or remain unscheduled). Recorded for honesty; none block the demo path.

- **`MockProvider` / `MockCodeDaemonClient` fallback in production is silent.** A
  misconfigured install can get fake answers with no warning. *(round-1 suggestion;
  round-2 arch failure-matrix.)*
- **GitHub hardwired in the wrong layer** — `parse_pr_url` lives in the review
  daemon's store and the extension has no platform-adapter indirection; GitLab is
  3-layer surgery. No `PlatformRef` extracted yet. *(round-2 arch erosion #4.)*
- **`worktree_path` cached on session rows has no invalidation** against future LRU
  eviction. *(round-2 arch failure-matrix.)*
- **Wrapper SIGKILL** can leave a live unsupervised review daemon that `start` then
  mislabels "already running". *(round-2 arch failure-matrix.)*
- **`AnthropicProvider::validate()` is not a live one-shot call** — it only checks
  the token is non-empty. (`/v1/config/validate` therefore confirms construction
  + a present credential, not a successful round-trip.) *(carried from round-1
  I24 intent; HTTP-level provider integration tests still thin.)*
- **Windows `send_term` graceful stop** still wastes the deadline then hard-kills.
  *(round-1 I20.)*
- **Log rotation** is still a TODO; logs grow unbounded. *(round-1 I21.)*
- **`SpawnedClient` reconnect/restart loop** still lightly covered. *(round-1 I23.)*
- **`MockCodeDaemonClient` tool/schema drift** vs the real daemon. *(round-1 I25.)*

Planned/future (never claimed as built): signed releases + notarization,
brew/scoop formulas, `libre-cr update` self-update, and OAuth review-posting
(spec Phase 9) all remain marked planned in `08-distribution.md` / `plan.md`.

---

## Full reports

- Round 1: [`../REVIEW/00-certification.md`](../REVIEW/00-certification.md)
  and the per-domain reports `../REVIEW/01-rust-core.md`,
  `../REVIEW/02-security.md`, `../REVIEW/03-tests.md`,
  `../REVIEW/04-frontend.md`, `../REVIEW/05-distribution-docs.md`.
- Round 2: [`../REVIEW/round2/00-certification.md`](../REVIEW/round2/00-certification.md)
  and `../REVIEW/round2/01-rust.md`, `../REVIEW/round2/02-extension.md`,
  `../REVIEW/round2/03-architecture.md`, `../REVIEW/round2/04-fix-verification.md`.
