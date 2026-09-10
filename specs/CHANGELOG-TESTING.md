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
  *Trigger: E1 / I16. Specs: 04 § Ask / streaming Q&A; 05 § Presentation Handler.*
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
- **The model explained the wrong line.** Asked about line 38 (a constant
  assignment), the answer described the lines *after* it and never mentioned the
  one selected. Two causes, both about what the model was handed: the wire
  carried only coordinates, so "line 38" meant nothing without counting, and
  `read_file` returned an unnumbered blob to count in — which it did, wrong.
  **Fixed:** every `Selection` variant carries an optional `text` field holding
  the selected code (populated on cell clicks, Cmd/Ctrl-click symbol picks and
  GitHub's own hash gestures, always from the clicked side), `build_user_message`
  quotes it back fenced and capped at 2,000 chars, and `read_file` now prefixes
  every line with its 1-based number (`   38 | …`). *Trigger: manual testing on
  PR #459 — "the line I selected is for this constant but the agent is talking
  about the lines after that". Specs: 10 § Evidence on the wire; 03 § File and
  structural reads.*
- **History replay carries recent tool results.** Replayed history was Q/A prose
  only, so a follow-up question lost the evidence its parent turn was grounded
  in and the model paraphrased from memory — in one traced turn it invented
  `_is_ambiguous` / `AmbiguousStorageError` (identifiers that exist nowhere)
  while the correct name sat in history prose. The last 2 turns now replay
  their tool results verbatim (20k chars/result, 40k/turn caps); older turns
  get a stub naming the tools used ("outputs no longer in context; re-read
  before citing"), and the system prompt now requires every cited identifier
  and file:line to appear verbatim in a currently visible tool result.
  *Trigger: manual testing on PR #459 — fabricated identifiers in a follow-up
  answer; diagnosed from `tool_traces`: the turn read only `retry.py` yet
  described `single_use_store.py` internals. Specs: 04 § Agent Loop.*
- **Copy from a rendered answer copies markdown.** Selecting text in the
  rendered panel and copying now serializes the selected fragment back to
  markdown (`fragmentToMarkdown`: emphasis, inline/fenced code with language,
  links, lists, tables, blockquotes, headings, task-list boxes) into
  `text/plain`, keeping the HTML flavor for rich-text targets. The sanitizer
  now preserves `language-*` classes on `<code>` so fences round-trip.
  *Trigger: manual testing — pasting a copied answer into a GitHub comment
  lost all formatting. Specs: 05 § Q&A Panel.*
- **Expanded turns ride along as context.** `AskInit` gained optional
  `context_turn_ids`: the panel sends the daemon ids of every turn the
  reviewer has left expanded, and the daemon replays those turns' tool
  results at full fidelity (in addition to the recency floor, capped at 5
  full-fidelity turns, oldest demoted to stubs first). Ids are matched only
  against the session's own turns, so a foreign id is inert. Restored turns
  carry their daemon id; live turns learn theirs from the `done` frame.
  Field is `#[serde(default)]` — older clients are unaffected.
  *Trigger: manual testing follow-up on the fabricated-identifier diagnosis —
  re-expanding an old turn is the natural "I'm asking about this" gesture.
  Specs: 04 § Agent Loop; 05 § Q&A Panel.*
- **CodeRabbit review round on PR #1.** Fixes: the `start` endpoint watcher
  only announces an endpoint file written after this start (a stale file that
  survived a failed removal printed a dead port); clone + prepare share one
  end-to-end 10-minute budget matching the UI's polling window; the stored-diff
  `get_pr_diff` fallback honors `paths`; `POST /v1/provider/models` never sends
  the *stored* key to an endpoint other than the one it was saved with (a
  candidate endpoint change requires its own key); the relayed `fetch` forwards
  a `Request` input's method/headers/body (was relayed as a bare GET); the
  export tool-log flag is markdown-only and the checkbox disables for GitHub
  reviews; cmd-click symbol picking reads the *clicked* code cell on React
  replacement rows; `open_link`'s description now states its real contract;
  "Clear highlights" is now "Clear all effects" everywhere; spec wording fixed
  (pairing auth exception, origin-check claims dropped for diagnostics-only,
  MD040 fence). The stale config-migration comment was verified already fixed.
  *Trigger: PR #1 review comments.*
- **CodeRabbit round two on PR #1.** Live `scroll_to` no longer moves the
  viewport (recorded only; tour/replay scroll — closes the loophole in the
  reviewer-initiated-scrolling rule); the `libre-cr-hide-labels` class is
  cleared on manager teardown and creation so reopened panels show captions;
  hash-decoded and clicked selections quote the *clicked side's* cell on
  replacement rows (`textOfLines` gained a side preference); stale async hash
  decodes are discarded by generation; the panel `max-height` accounts for its
  80px top offset; `highlight_lines` marks `label`/`detail` required in the
  schema, matching its description; `/v1/provider/models` counts only a
  *string* `api_key` as explicit (null no longer skips key-clearing); history
  replay budgets by messages, not turns (a turn is two messages); the ledger's
  superseded replay entry says so. The new live-scroll regression test caught a
  real bug beyond the review: `clear_presentation` *removed* flash-tagged
  elements, and `scroll_to` tags GitHub's own row as flash — so clearing after
  a scroll deleted the diff row from the page. Flash rows are now stripped like
  highlights; only our inserted annotation rows are removed.
  *Trigger: PR #1 review comments.*
- **A wide `highlight_lines` froze (and killed) the tab.** Asked to point at a
  class, the model called `highlight_lines` with `start_line: 136,
  end_line: 614` — a 479-line span. `highlightLines` looped every line calling
  `findRow`, and `findRow` re-resolves the file container with a
  document-wide `querySelectorAll` + `Array.from` + `filePathOf` map on every
  call, so the range cost O(range × document) of synchronous DOM work plus 479
  React-visible row mutations. **Fixed:** new `findRows(file, start, end)`
  resolves the container once and scans it once (479 scans → 1), the span is
  clamped to `MAX_HIGHLIGHT_LINES = 80` with the clamp reported back to the
  model in the `presentation_result`, and the tool description now tells the
  model to make several narrow highlights rather than one sweeping range.
  *Trigger: manual testing on PR #474 — the tab crashed mid-walkthrough.
  Specs: 09 § highlight_lines.*
- **`libre-cr stop` stopped the wrong process; `start` called an orphan
  healthy.** Both commands acted on `run/review.pid`, which the supervisor
  fills with its *child's* PID (`supervisor.rs:190`). So `stop` SIGTERMed the
  review daemon inside a live restart loop — the supervisor respawned it
  ~250 ms later and the next `start` said "already running" — while a
  SIGKILLed wrapper left an unsupervised daemon that `start` read as a healthy
  install and refused to replace, even though it was holding the port.
  **Fixed:** the supervisor now records itself in `run/supervisor.pid`, which
  is what `stop` signals (its SIGTERM handler stops the child gracefully and
  leaves the loop) and what `start`/`status` treat as "running". `stop` also
  reaps a daemon that outlived its supervisor, and `start` clears an orphan
  before binding; `status` reports the two separately and flags
  "running unsupervised". Escalation (TERM → wait → KILL) is now one shared
  `proc::terminate_and_wait`, and both commands take their pid-file paths as
  arguments so the tests don't depend on the ambient `$HOME` the integration
  suite re-points concurrently. Verified live: `stop` now leaves no process,
  a closed port, and no pid files.
  *Trigger: taking over daemon management during manual testing.
  Specs: 08 § Wrapper CLI Surface, § Supervision Model.*
- **A long conversation outgrew the model's context, and the failure left no
  trace.** After 7 turns on one PR the session stopped answering. Diagnosed
  from stored sizes, because nothing was logged: `kimi-k2.6` has a
  262,144-token context, one `get_pr_diff` without `paths` had returned
  **589,499 chars** (~168k tokens) in a single result, turn 1's cumulative
  `usage_in` was **760,326** tokens across its rounds, and live tool results
  were fed to the model completely uncapped (`loop_.rs` pushed
  `outcome.value.to_string()` straight into the message array). The
  history-replay feature added days earlier made it worse by design: it raised
  the per-ask floor from ~9k tokens of prose to as much as ~57k of replayed
  tool output. **Fixed:** every live tool result is capped
  (`max_tool_result_chars`), a shared per-turn budget bounds all of them
  together (`max_turn_tool_chars`), and the replay caps became config too.
  Exceeding a cap never fails the turn — the result is truncated, the model is
  told how to narrow its next call (`paths`, `start_line`/`end_line`), and a
  floor guarantees it always receives a readable head so the loop can finish.
  All eight caps are editable in the daemon's own config UI (`/config-ui`,
  the page the popup's "Configure daemon" link and `libre-cr config` open) —
  one form, one Save, sending a partial range-checked `limits` patch to
  `POST /v1/config`. They are deliberately *not* in the extension's options
  page: the daemon enforces them and must keep them without the extension, and
  splitting config across two editors is the leak the spec already warned
  about. The `tool_result` frame carries `truncated_from`, and the panel shows
  a per-turn notice plus a per-trace marker so a shortened answer is never
  silent. Separately, a failed turn now
  logs at error level and persists an `error` row — previously it produced no
  log line and no row, which is why this had to be reconstructed from
  arithmetic. *Trigger: manual testing — "this long discussion is now
  systematically crashing". Specs: 04 § Configuration, § Configuration UI.*
- **`get_pr_comments` now returns actual comments.** The tool read
  `pr_data.comments`, a field the scraper never wrote, so it had answered
  `{ comments: [] }` to every call since it shipped (see § Still open, where it
  was first filed). The extension now extracts review comments from the page's
  embedded JSON payload — `markers.threads` joined with each
  `diffSummaries[].markersMap` for the anchor — and each comment carries
  `file`/`line`/`side` (`start_line` for a range), its thread id, its
  `databaseId`, and its thread's `resolved` state, so the model can read the
  code a concern points at and can tell a settled thread from a live one.
  Replies are flattened into rows sharing a `thread_id`.

  The **DOM was the wrong source**, for two reasons measured on live PRs.
  Threads are virtualized: on our own PR exactly **1 of 29** was in the DOM.
  And the changes UI renders only *unresolved* threads at all — a PR with 13
  resolved reply threads had **none** in the DOM with the annotated file on
  screen, and the one thread our PR did render was its one unresolved anchored
  thread. The payload holds all of them and is already inside the reviewer's
  authenticated session, which the daemon is not (no GitHub token; OAuth
  posting is unbuilt). REST via a token remains the better source if one ever
  exists — filed as v2, not built.

  **A first cut of this dropped more than it kept.** It required a `file` and
  `line` on every comment, reasoning that a concern the model cannot locate is
  not usable evidence. Measuring the payload afterwards showed **17 of 29**
  threads have no anchor in `markersMap` — GitHub only maps lines that survive
  in the current diff — and one of those 17 was *unresolved*. So each comment
  now carries an `anchor` of `line`, `file` (a file-level comment) or `none`,
  and nothing is dropped for want of a location. The lesson is the same one
  this ledger keeps recording: the assumption was reasonable, the measurement
  disagreed, and only the measurement counts.

  Capture is bounded (100 comments, 1,200 chars per body) because `pr_data` is
  stored in the session row and re-sent on every init; `truncated` reports our
  cap *or* GitHub's own `threadsPageInfo.hasNextPage`. When the payload cannot
  be parsed the field is omitted and the tool answers `unavailable: true` —
  "unknown", never "none", which is the failure the old behaviour had.
  Measured on PR #1: 86 files, 29 threads (12 anchored, 17 not), 27 resolved,
  bodies 1.4k–3.6k chars.
  *Trigger: found while designing review-comment selection; fixed in its own PR
  first. Specs: 04 § Internal Tools; 05 § Review-comment selection.*

- **Review-comment selection.** A reviewer can hover a review thread in the
  diff, click "Ask about this", and ask a question with the thread as context:
  the whole thread (replies included), plus the `file`, `line` and diff `side`
  it annotates. The daemon quotes it as `@author: body` blocks under a
  `[Selection: review comment on line N (old|new side) in <file>]` header, so
  the model gets the concern *and* where it points, and can go read that code.

  The gesture is a hover affordance rather than a modifier-click: a comment
  body is full of links, `Reply` and `Resolve`. The button is a single element
  on `documentElement`, positioned `fixed` from the thread's rect — outside
  React's tree, so a re-render cannot strip it and nothing needs re-injecting.

  Two things the spec had flagged as designed-but-unobserved were checked on
  live PRs first. A thread with replies behaved exactly as designed. A resolved
  thread turned out not to exist in the DOM at all — the changes UI renders
  only unresolved threads — so selection reaches unresolved threads only, and
  `Selection` carries no resolved state. A third case the spec had missed,
  file-level comments, renders as a thread with no diff row and therefore no
  anchor; the affordance computes the selection on hover and stays hidden when
  there is none, rather than offering a control whose click does nothing.
  *Trigger: requested feature, specified then built. Specs: 05 § Selection
  Model, § Review-comment selection; 04 § Ask; 10 § Evidence on the wire.*

---

## Findings log (certification flags and field bugs, in order found)

> This log records *what changed and why*. The contract that resulted from the
> grounding and context rounds is stated in `10-grounding-and-context.md`;
> where a fix changed a documented behaviour, the entry names the spec section
> it landed in.

Every item the certification rounds or manual testing flagged, kept with its
full diagnosis. Most were fixed in place and say so (**Fixed**, or describe the
shipped change); the **Still open / deferred** block at the end collects what
remains deliberately unfixed. None of the open items block the demo path.

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
  testing Tier 2. Specs: 02 transports; 04 § HTTP / WebSocket API, § Pairing; 05
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
  first clone. *Trigger: manual testing Tier 3. Specs: 04 § Internal
  Architecture (worktree orchestration); 05 § Content Script Lifecycle.*
- **First clone of a real repo hit the 10 s code-daemon call timeout.**
  `SpawnedClient` applied one `CALL_TIMEOUT` (10 s) to every call; a 300 MB
  clone took longer, the review daemon reported "clone failed: code daemon
  call timeout" while the clone completed underneath. **Fixed:**
  `call_with_timeout` on the client trait; `clone_repo` / `prepare_worktree`
  get 10 min, tool calls keep 10 s. *Trigger: manual testing Tier 3, private
  repo. Specs: 04 § Internal Architecture (worktree orchestration).*
- **Presentation effects were invisible.** `highlight_lines` tagged rows with
  `libre-cr-effect libre-cr-hl-<color>` and `annotate_line` inserted rows, but
  no stylesheet anywhere defined those classes — the only `<style>` lives in the
  panel's shadow root and can't reach GitHub's rows. Users saw nothing (or bare
  unstyled annotation text), so "Clear all" looked like a no-op even though it
  cleared correctly (verified live: DOM markers and counters reset). **Fixed:**
  page-level effect CSS installed via `adoptedStyleSheets` (CSSOM insertion is
  outside GitHub's `style-src` CSP), and the footer button renamed so it isn't read as
  clearing the conversation — "Clear highlights" at the time, and "Clear all
  effects" since the CodeRabbit round below, because it also clears annotations
  and flashes. *Trigger: manual testing Tier 3. Specs: 09 § Extension
  Implementation.*
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
  called; `get_pr_diff` is computed by the router via `git_diff` with
  `merge_base: true` — a three-dot `origin/<base>...HEAD` — on the session
  worktree (optional `paths`); two-dot is the notation this fix rejected, since
  it attributes base-branch commits to the PR; the system
  prompt states the checkout path and base branch and that code tools already
  operate there. *Trigger: manual testing Tier 3. Specs: 04 § Agent Loop,
  § Tool Composition Per Verb (in 06).*
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
  Replay (clear, then re-apply steps 0..k). Initial implementation — the paced
  replay was later superseded by the reviewer-driven guided tour (see "Guided
  tour replaces timed replay" below). *Trigger: manual testing.*
- **Highlights were hard to relate to the answer.** The `label` the model
  passes with `highlight_lines` was only a hover `title`. It is now a caption
  chip floating at the right end of the range's first code line (attribute-keyed
  CSS, so GitHub re-renders leave it alone), with a "Captions on/off" footer
  toggle; the prompt/tool description make the label mandatory and tied to the
  answer's heading, and ask for a closing `scroll_to` on the first part. The
  replay widget scrolls to each step's row as you step. *Trigger: manual testing
  — first clean 7-highlight walkthrough, but no scroll and no way to map chips
  to sections.*
- **The phantom CDK rollback came back — from history.** After the three-dot
  fix `get_pr_diff` was correct, but the agent loop replays prior Q&A and the
  model repeated its own earlier (wrong-diff) section over the fresh tool
  output. Prompt now states that current tool results outrank earlier answers.
  Sessions answered before the fix stay contaminated; the test session was
  deleted so the reviewer starts clean. Deeper fix (not done): store the tool
  results a turn relied on and drop/flag turns whose inputs changed.
- **`session_history_search` crashed on ordinary queries** (`no such column:
  33392`): raw text went straight into FTS5 `MATCH`, where `-` and `"` are
  syntax. Tokens are now individually quoted (`fts_query`). *Trigger: manual
  testing trace.*
- **`scroll_to` and replay stepping didn't visibly scroll; Replay jumped to the
  end.** `scrollIntoView({behavior: "smooth"})` is cancelled by any other scroll
  or layout shift, and GitHub's virtualizer shifts layout as rows mount, so the
  call "succeeded" and nothing moved. Now `scrollIntoViewSettled`: instant
  scroll, re-check the row is in the viewport, retry up to 4×. Replay is a paced
  tour (step, pause 1.8 s, step…; click again to stop) instead of applying
  everything and landing on the last step. *Trigger: manual testing —
  11-highlight walkthrough, `scroll_to` ok but no movement, "steps 7-8 and
  nothing".*
- **Guided tour replaces timed replay.** Auto-paced replay was too fast and the
  footer controls too small; relating a highlight to its explanation still
  meant reading the transcript. `highlight_lines` gains `detail` (the model's
  1–3 sentence explanation, required alongside `label` for walkthroughs); the
  panel offers a **Tour (N)** button opening a floating widget (bottom-centre,
  big Prev/Next, counter, title, file:line, explanation, Show all, close) that
  shows one step at a time (`showStep`: clear → apply → verified scroll).
  Follow-up: the assistant's first presentation call of a turn opens the widget
  *armed* — one big "Scroll to first highlight" button, nothing scrolls until
  the reviewer clicks; opening from the footer goes straight to the controls.
  Live effects never scroll.
  *Trigger: manual testing — "timing is brutal", "the AI's text should be there".*
- **A daemon dying mid-answer looked like an empty answer.** `AskSession`
  resolved a WebSocket close after open as a normal completion even when no
  `done` frame arrived, so the panel showed "A:" with nothing and no error. Now
  a close without `done` rejects ("connection closed before the answer
  completed"); the export marks cancelled/failed turns. *Trigger: manual testing
  — the daemon was reaped by the launcher mid-turn (see supervisor lifetime).*
- **Selection now rides GitHub's own line selection.** Reviewers reach for
  GitHub's native gesture (click a line number = one line, shift-click =
  range, with native row highlighting); ours only understood clicks on cells.
  GitHub publishes its selection in the URL hash as `#diff-<sha256(path)><R|L><a>[-<R|L><b>]`
  (digest scheme verified live), so `watchGithubLineSelection` decodes
  hashchange events into line/range selections — multi-line selection for
  free, no gesture ownership. Direct cell clicks and cmd-click symbols still
  work. Follow-up: GitHub sets the hash via history.pushState (no hashchange
  event), and the legacy shift-click branch was overwriting the range with
  {last,last} — the watcher now re-checks the hash after every click and the
  pseudo-range branch is gone. *Trigger: manual testing — "can we hook into
  GitHub's selection?"; range showed the last-clicked line twice.*
- **Panel UI round, after examining GitHub Copilot's "Explain" panel.** Worth
  copying from it: markdown-rendered answers and per-exchange reference tokens;
  not copied: its docked form factor (floating retained; docked/fixed modes for
  small screens deferred). Added: sanitized markdown rendering (`marked` behind
  an allowlist sanitizer — model output must never reach innerHTML raw), a
  selection chip on each question ("src/a.rs:34-36"), and a natively resizable
  shell (CSS `resize: both`, size persisted per PR next to the drag position).
  *Trigger: manual testing — plain-text answers, fixed panel size.*
- **Conversation history now restores; empty panels stay closed.** The daemon
  stored every turn (the export reads them) but the panel started empty on
  every page load. It now rebuilds the conversation from `GET /v1/sessions/:id`
  (collapsed Q&As with selection chips, editable notes, cancelled/failed
  markers) — and a session with no history starts closed behind the floating
  CR button; errors and not-paired still auto-open. *Trigger: manual testing —
  "refresh wipes the conversation".*
### Still open / deferred

- **Reloading the extension orphans open tabs' content scripts** (also logged
  above with its diagnosis) — detection of `runtime` loss and a "reload this
  page" notice are still to do.
- **Reloading the unpacked extension wipes `storage.local` → re-pair.** Every
  dev reload of the extension forces a new pairing (and a new 5-minute code).
  Folds into the pairing-UX item above. *Trigger: manual testing.*
- **The supervisor still runs in the foreground**, so the daemons live and die
  with whatever launched them — during testing a launcher reaping its children
  after hours idle took the review daemon down twice (`graceful-stop`, no
  crash). A `libre-cr start --detach` (own session, `setsid`-style) or a
  launchd/systemd unit at distribution time is the real fix. (The `stop`
  half of this item is fixed; see the findings log.)
  *Trigger: manual testing — idle kills.*
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
- **`AnthropicProvider::validate()` is not a live one-shot call** — it only checks
  the token is non-empty. (`/v1/config/validate` therefore confirms construction
  + a present credential, not a successful round-trip.) *(carried from round-1
  I24 intent; HTTP-level provider integration tests still thin.)*
- **Windows `send_term` graceful stop** still wastes the deadline then hard-kills.
  *(round-1 I20.)*
- **Log rotation: decided against.** Logs grow unbounded — the daemons write to
  stderr and the supervisor appends that stream to disk with no rotation or
  retention. Not a defect to fix: for a local single-user tool `libre-cr logs`
  plus manual deletion is the accepted answer. The specs claimed daily rotation
  with 14-day retention and now say this instead. *(round-1 I21, closed as
  won't-do; spec audit finding 7.)*
- **No graceful shutdown in the review daemon.** It installs no signal handler,
  so a `SIGTERM` ends the process abruptly: in-flight turns are not marked
  `cancelled`, and SQLite is not flushed deliberately. Per-connection
  cancellation *is* implemented, and the supervisor gives the daemon 5 s before
  `SIGKILL` — time it currently spends doing nothing. The specs claimed the
  drain and now mark it unbuilt. *(spec audit finding 27.)*
- **`clone_repo` has no containment check, and no daemon checks config file
  mode.** The specs asserted both as enforced. `target_dir` is tilde-expanded
  and used verbatim, so a caller naming a path outside `data_dir` is honoured;
  nothing inspects config permissions. Exposure is bounded — the tool is hidden
  from the model, so it takes a caller holding the bearer token — but the
  containment check is cheap and worth having. *(spec audit findings 6.)*
- ~~**`get_pr_comments` has always returned an empty list.**~~ **Fixed** — see
  § Manual-testing changes. The scraper now populates `pr_data.comments` from
  the embedded page payload. Still open from the same finding: reading comments
  through the REST API instead of the page, which needs a GitHub token the
  daemon does not have (**v2**).
- **`SpawnedClient` reconnect/restart loop** still lightly covered. *(round-1 I23.)*
- **`MockCodeDaemonClient` tool/schema drift** vs the real daemon. *(round-1 I25.)*

Planned/future: signed releases + notarization, brew/scoop formulas,
`libre-cr update` self-update, and OAuth review-posting are all unbuilt.
`plan.md` marks them planned. `08-distribution.md` does **not** — it presents
several of them as shipped, which the spec audit recorded as a finding of its
own rather than something this ledger can claim is documented correctly.

---

## Full reports

- Round 1: [`../REVIEW/00-certification.md`](../REVIEW/00-certification.md)
  and the per-domain reports `../REVIEW/01-rust-core.md`,
  `../REVIEW/02-security.md`, `../REVIEW/03-tests.md`,
  `../REVIEW/04-frontend.md`, `../REVIEW/05-distribution-docs.md`.
- Round 2: [`../REVIEW/round2/00-certification.md`](../REVIEW/round2/00-certification.md)
  and `../REVIEW/round2/01-rust.md`, `../REVIEW/round2/02-extension.md`,
  `../REVIEW/round2/03-architecture.md`, `../REVIEW/round2/04-fix-verification.md`.
