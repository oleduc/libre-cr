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

- **BUG — content-script CORS: the extension cannot reach the daemon from a PR
  page.** `05-browser-extension.md` § Transport from a Content Script assumes a
  content script's `fetch` carries the extension origin. It does not (Chrome ≥ 85 /
  MV3): it carries the *page* origin, `https://github.com`. Pairing persists
  `chrome-extension://<id>` as the sole CORS allow-origin, so every daemon call
  from the CR panel fails with `transport: Failed to fetch`. The browser E2E is
  green only because its harness sets `extension_origin = "https://github.com"`
  (`e2e-browser/helpers/daemon.ts:35-41`) — the test encodes the workaround
  instead of the production behaviour. **Fixed in two steps.** (1) CORS is
  now `*` and the auth middleware's origin check is gone — the bearer token is
  the boundary; the unauthenticated routes (`/v1/health`, rate-limited
  `/v1/pair`) were reachable by anything on the machine via curl regardless.
  `extension_origin` is still persisted on pair, for diagnostics only.
  (2) That was necessary but not sufficient: a content script's `fetch` also
  inherits the page **CSP**, and github.com's `connect-src` excludes
  127.0.0.1 — Chrome fired `securitypolicyviolation` and the request never
  left the browser. Daemon traffic from the content script is now relayed
  through the background service worker (`utils/daemon/proxy.ts` +
  `entrypoints/background.ts`) via the `fetch` / `wsFactory` injection points
  the client already had; the browser-E2E fixture page now carries
  `connect-src 'self'` so the suite exercises this for real. *Trigger: manual
  testing Tier 2. Specs: 02 transports; 04 § HTTP API, § Pairing; 05
  § Transport from a Content Script, § Background Service Worker.*
- **GitHub's React "changes" UI broke every DOM selector.** github.com now
  redirects `/pull/<n>/files` → `/pull/<n>/changes`, a React page with none of
  the classic hooks: no `.gh-header-title`, no `.base-ref/.head-ref`, no
  `td.blob-num`, no `[data-tagsearch-path]`, no head-SHA `<meta>`. The panel
  showed "missing title / missing base-head — selectors may need refresh", and
  line selection + highlights silently found nothing. **Fixed:** selectors now
  cover both DOMs (`h1 .markdown-title`; `a[class*=PullRequestBranchName]` in
  base→head order; `table[aria-label="Diff for: <path>"]`;
  `td[data-line-number][data-diff-side]`; head SHA from the
  `react-app.embeddedData` JSON's `headOid`). `SELECTOR_VERSION` → 2; new
  fixture test `tests/github-react-ui.test.ts`. *Trigger: manual testing Tier 2.
  Specs: 05 (GitHub adapter and selectors).*
- **Typing in the panel triggered GitHub hotkeys ("t" focused GitHub search).**
  Shadow-DOM retargeting: a keystroke in the panel's textarea reaches
  `document` with `target` = the `#libre-cr-root` host, so GitHub's hotkey
  handler sees a non-editable target and fires. **Fixed:** the host stops
  `keydown`/`keypress`/`keyup` propagation at the shadow boundary; React's own
  listeners sit inside the shadow and are unaffected. *Trigger: manual testing
  Tier 2. Specs: 05 § Content Script Lifecycle.*
- **Mock provider answered only the first question.** `MockProvider` consumed
  its script one burst per `stream()` and then returned an empty stream, which
  the agent loop reports as `internal: provider stream ended without done` on
  the second ask. Correct for multi-burst unit scenarios, a trap for the
  documented keyless Tier 2 flow. **Fixed:** the queue refills from the script
  when exhausted (empty scripts stay empty). *Trigger: manual testing Tier 2.
  Specs: 04 § LLM Provider Layer (mock).*
- **Provider `endpoint` wanted the full request URL while the docs promised a
  base URL.** `openai_compat` POSTed to the string verbatim, so the documented
  `http://127.0.0.1:11434/v1` (and OpenRouter's `https://openrouter.ai/api/v1`)
  would 404, and the derived `/models` URL was wrong too. **Fixed:** both
  providers accept a `/v1` base (appending `/chat/completions` or `/messages`)
  or the full path. *Trigger: manual testing Tier 3 (OpenRouter). Specs: 04
  § LLM Provider Layer; docs configuration.md.*
- **Worktree never became ready — for every repo.** Two gaps stacked: (1) the
  orchestrator required `pr_data.remote_url`, which the extension never sends
  (it scrapes only the slug), so prep failed instantly with "session has no
  remote_url in pr_data"; (2) even with a URL, a discovery miss ended in
  `clone_required` — "extension should prompt to clone (Phase 5)" — and the
  extension has no such prompt, while the code daemon's `clone_repo` tool sat
  unused. Meanwhile the panel polled `worktree_ready` only, ignored
  `status.error`, and after 60 s said "Worktree never became ready".
  **Fixed:** remote URL derived as `https://github.com/<owner>/<repo>.git`;
  discovery miss → `clone_repo` into the managed cache → `prepare_worktree`;
  the panel stops on `status.error` and shows it, and waits long enough for a
  first clone. *Trigger: manual testing Tier 3. Specs: 04 § Worktree
  orchestration; 05 § Content Script Lifecycle.*
- **First clone of a real repo hit the 10 s code-daemon call timeout.**
  `SpawnedClient` applied one `CALL_TIMEOUT` (10 s) to every call; a 300 MB
  clone took longer, the review daemon reported "clone failed: code daemon
  call timeout" while the clone completed underneath. **Fixed:**
  `call_with_timeout` on the client trait; `clone_repo` / `prepare_worktree`
  get 10 min, tool calls keep 10 s. *Trigger: manual testing Tier 3, private
  repo. Specs: 04 § Worktree orchestration.*
- **Presentation effects were invisible.** `highlight_lines` tagged rows with
  `libre-cr-effect libre-cr-hl-<color>` and `annotate_line` inserted rows, but
  no stylesheet anywhere defined those classes — the only `<style>` lives in the
  panel's shadow root and can't reach GitHub's rows. Users saw nothing (or bare
  unstyled annotation text), so "Clear all" looked like a no-op even though it
  cleared correctly (verified live: DOM markers and counters reset). **Fixed:**
  page-level effect CSS installed via `adoptedStyleSheets` (CSSOM insertion is
  outside GitHub's `style-src` CSP), and the footer button renamed "Clear
  highlights" so it isn't read as clearing the conversation. *Trigger: manual
  testing Tier 3. Specs: 09 § presentation handler.*
- **Closing the panel left no way to reopen it** — `ContentApp` rendered
  nothing when closed. **Fixed:** a small fixed "CR" reopen button remains.
  *Trigger: manual testing.*
- **Line selection did nothing on the React "changes" UI.** `SelectionLayer`
  gated clicks on `[data-tagsearch-path]` and `td.blob-num/.blob-code`.
  **Fixed:** it uses the shared dual-DOM selectors from `utils/github/diff.ts`;
  `hitTestLine` now honours the clicked cell's own line/side (React UI code
  cells carry `data-line-number`/`data-diff-side`) instead of always taking
  the row's first (old-side) number. *Trigger: manual testing.*
- **Presentation tools vs. GitHub's virtualized diff.** The React "changes" UI
  renders only files near the viewport (`data-estimated-height` placeholders,
  progressive list); `findRow` returns nothing for the rest, so
  `highlight_lines` / `scroll_to` on a file below the fold fail with
  `file_not_in_view`. Also observed: GitHub's React rewrites row `className` on
  hover/selection, wiping effect *classes* — effects are now keyed on `data-*`
  attributes (`data-libre-cr-tag`, `data-libre-cr-color`) which React leaves
  alone. Tool descriptions now tell the model when to highlight, that lines are
  NEW-side numbers, that `scroll_to` needs a line inside a hunk, and that
  `get_pr_diff` is normally empty (the extension never scraped hunks) so it
  should use `git_diff` on the worktree. **Then fixed for real:** traces showed
  every failure was one file (`cdk/lib/cdk-stack.ts` → `file_not_in_view`)
  while highlights on rendered files succeeded. The extension now forces
  GitHub to mount a file before targeting it (`ensureFileRendered`: scroll the
  file's placeholder region / click its file-tree link, wait for rows);
  `scroll_to` without a line scrolls to the file header; the prompt tells the
  model to continue past a file that can't be shown.
  *Trigger: manual testing Tier 3 — "scroll to top, no highlights".*
- **Reloading the extension orphans open tabs' content scripts** — the panel
  shows "Extension context invalidated" and every call fails until the page is
  reloaded. Should detect `runtime` loss and show "reload this page".
  *Trigger: manual testing (dev loop).*
- **The model went repo-hunting and called `clone_repo` itself.** With
  `get_pr_diff` always empty and no hint of where the checkout was, a turn ran
  `discover_repo` → `scan_for_repos` → `clone_repo` with a guessed URL, which
  failed on the private repo, and the answer degraded to "I don't have access
  to the diff". **Fixed:** worktree-management tools (`clone_repo`,
  `discover_repo`, `scan_for_repos`, `prepare_worktree`, `list_worktrees`,
  `remove_worktree`) are no longer offered to the model and are refused if
  called; `get_pr_diff` is computed by the router via `git_diff
  origin/<base>..HEAD` on the session worktree (optional `paths`); the system
  prompt states the checkout path and base branch and that code tools already
  operate there. *Trigger: manual testing Tier 3. Specs: 04 § Agent Loop,
  § Tool Composition.*
- **Export "tool call log" option.** Diagnosing presentation failures needed
  the tool inputs/results, which the export only summarised as `name (ms, ok)`
  — "ok" there is transport, not the tool's outcome — so they had to be read
  from SQLite by hand. `ExportFilter.include_tool_io` renders each trace's
  input and result as JSON (capped per value); the export modal has an
  "Include tool call log" checkbox (context/transcript modes).
  *Trigger: manual testing — debugging highlights.*
- **`get_pr_diff` reported changes that aren't in the PR.** The router used a
  tip-to-tip diff (`origin/main..HEAD`); `main` had gained commits since the PR
  forked, so they appeared as "deleted in the PR" (14 files vs GitHub's 9 — the
  phantom CDK-table rollback). **Fixed:** the code daemon's `git_diff` gained
  `merge_base` (three-dot `origin/<base>...HEAD`), which the router always sets;
  the model's own `git_diff` calls can use it too. *Trigger: manual testing —
  reviewer spotted a file not in the PR.*
- **Walkthroughs came back as text only.** With "use them sparingly" in the
  prompt, a "walk me through the important parts" turn produced one `scroll_to`
  and no highlights. **Fixed:** the prompt now says highlighting each described
  part *is* the deliverable for walk-through / point-out / show requests.
- **Presentation replay widget.** The model fires its highlights/scrolls while
  streaming, so the reviewer only ever sees the end state — and testing
  presentation required a paid model call each time. The presentation manager
  now records each successful call as a step; the panel footer gets ◀ k/N ▶ and
  Replay (clear, then re-apply steps 0..k). *Trigger: manual testing.*
- **Reloading the unpacked extension wipes `storage.local` → re-pair.** Every
  dev reload of the extension forces a new pairing (and a new 5-minute code).
  Folds into the pairing-UX item above. *Trigger: manual testing.*
- **BUG — `libre-cr stop` does not stop the supervisor.** `stop` SIGTERMs the PID
  in the pid file, which is the *review daemon* child; the supervisor logs
  `unclean-exit code=None` and respawns it 250 ms later (on a fresh ephemeral
  port), and the next `start` reports "already running". `stop` must target the
  supervisor, or the supervisor must treat a stop-requested TERM as intentional.
  Related: the supervisor runs in the foreground, so the daemons live and die
  with whatever launched them — during testing a launcher reaping its children
  after hours idle took the review daemon down twice (`graceful-stop`, no
  crash). A `libre-cr start --detach` (own session, `setsid`-style) or a
  launchd/systemd unit at distribution time is the real fix.
  *Trigger: manual testing — restart after a config edit; idle kills.*
- **Pairing UX: one-time code + 5-minute TTL is a bad experience.** Manual
  testing: the code expired before the extension was loaded and the options
  form filled in (endpoint must also be re-typed — the form defaults to
  `:8765` while the daemon picks an ephemeral port). Needs an easier path:
  e.g. `libre-cr pair` prints/opens the existing auto-pair deep link
  (`#pair?endpoint=…&code=…&auto=1`) once the extension origin is known, a
  fixed default port so the endpoint never needs typing, or an inverted flow
  where the extension requests and the CLI approves — no code to copy at all.
  Interim: `libre-cr pair` now requests the 15-minute maximum TTL.
  *Trigger: manual testing — pairing said "unauthorized" on an expired code.*
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
