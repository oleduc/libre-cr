# Spec Change Record

What every specification claim said before the September 2026 reconciliation,
and what it says now. Sibling to `CHANGELOG-TESTING.md`: that file records
changes to the **code** found by testing; this one records changes to the
**specs** found by auditing them against the code.

Range: `402652f..` on branch `specs-grounding-and-context`. The full text of
any change is in git; the quotes below are trimmed to the claim.

## When

The reconciliation happened on **2026-09-10**, in four commits:

| Commit | Time (local) | What landed |
|---|---|---|
| `697fb38` | 10:32 | `10-grounding-and-context.md` written; specs 01, 03, 04, 05, 08, 09 brought up to date for the testing rounds; the missing ledger entry added |
| `a82c608` | 10:45 | The four audit findings in text written during those rounds |
| `91f07a3` | 12:01 | The remaining 59 findings, under the agreed triage |
| `8471536` | 15:51 | This record |

The spec audit itself ran on 2026-09-10 between the first and third commits;
its findings are what the third commit applies.

**Granularity:** every row in the tables below landed in the commit named
above for its disposition — the ~90 changes were not made incrementally over
time, so a per-row date would be false precision. `git log -p` on the file is
the authority for anything finer.

### How long the drift had accumulated

Every specification was written in a single design pass on **2026-06-23**,
before any code existed. "Last touched" below counts only the in-place patches
made during the August–September testing rounds; none of those was an audit.

| Spec | First written | Last touched before | Days unreconciled |
|---|---|---|---|
| `01-overview.md` | 2026-06-23 | 2026-08-26 | 15 |
| `02-architecture.md` | 2026-06-23 | 2026-09-01 | 9 |
| `03-code-daemon.md` | 2026-06-23 | 2026-08-26 | 15 |
| `04-review-daemon.md` | 2026-06-23 | 2026-09-01 | 9 |
| `05-browser-extension.md` | 2026-06-23 | 2026-09-01 | 9 |
| `06-investigation-verbs.md` | 2026-06-23 | 2026-06-23 | 79 |
| `07-conversation-and-notes.md` | 2026-06-23 | 2026-06-23 | 79 |
| `08-distribution.md` | 2026-06-23 | 2026-08-26 | 15 |
| `09-presentation-tools.md` | 2026-06-23 | 2026-09-01 | 9 |

Two documents — `06-investigation-verbs.md` and `07-conversation-and-notes.md`
— had not been edited at all since the day they were written, 79 days earlier.
They produced 7 and 5 findings respectively. The four touched most recently
still produced findings, because those touches were feature patches, not
checks: editing a document is not the same as verifying it.

The code the specs now describe spans **2026-08-26 to 2026-09-10** — 39
commits ahead of `main` across `post-certification-fixes`,
`context-limits-and-lifecycle-fixes` and this branch.

## Why this reconciliation happened

`specs/01`–`09` were written **before the code existed**, on 2026-06-23 — the
day of the repository's initial commit. They are design documents, and for the
79 days since they were read as descriptions of a running system. Nobody had
checked them against the implementation.

An agent fact-checked all ten specs plus the testing ledger against `crates/`
and `extension/`: **~515 claims checked, ~410 confirmed correct, 63 findings**
(23 HIGH, 23 MEDIUM, 17 LOW). The trigger was finding a fabricated
cross-reference by hand while writing `10-grounding-and-context.md` — a
citation to a ledger entry that did not exist — which implied others.

The findings clustered almost entirely in the **unbuilt** sections. That is
the expected fate of a design document: its predictions about things nobody
built are the ones that rot. The parts that got built and stayed accurate
stayed accurate because they were decisions that got honoured — the
two-daemon split, MCP between them, the API key never reaching the extension,
presentation tools as a secondary capability. Those held for the whole 79 days,
through 39 commits of implementation.

So the defect was not bad design. It was reading a design document as a
description, with nothing in the text to tell the two apart.

### The distinction this record exists to preserve

There are now two kinds of text in these specs, and they fail in opposite
ways:

| | Written **before** the code | Written **after** the code |
|---|---|---|
| What it is | A design decision | A description of behaviour |
| How it fails | **Drift** — says X, code does Y | **Laundering** — makes whatever the code happens to do look deliberate |
| Detectable by | Exactly the audit above; comparison finds it | Nothing. Comparison to the code always passes |

`10-grounding-and-context.md` was written *after* the code, from reading it.
It came back from the audit with zero findings — which is close to
unfalsifiable, since it was verified against the same source it was
transcribed from. Consistency of a transcription is not evidence of a sound
design.

The test for whether a line in these documents earns its place: **could the
code be changed to violate it, and would that change be wrong?** If yes, it is
a specification. If the answer is "then the doc is merely stale", it is
documentation and belongs next to the code. Spec 10's normative claims (a cap
never fails a turn; the progress floor exists; the grounding rules; the store
keeps the whole result) pass that test. Its recitation of cap *values* does
not — those duplicate `[limits]` and are a second place to go stale.

### Status markers

Unbuilt sections now carry an explicit marker rather than being deleted or
reworded, so the design survives while the reader can tell it is not
description:

```text
> **Status: not built.** <what actually happens today, with the evidence>
```

## Dispositions used below

| Label | Meaning |
|---|---|
| **Corrected** | The spec contradicted shipped code. The code was right. |
| **Not built** | Design text kept, status marker added. Still on the plan. |
| **Dropped** | Aspiration removed; the specs now describe what happens and why that is acceptable. |
| **Filed** | A real gap the spec had asserted as done. Moved to the ledger's backlog. |
| **New** | Content that did not exist before. |

---

## `01-overview.md`

| Was | Now | |
|---|---|---|
| Code intelligence (phase B): "ast-grep + ripgrep + tree-sitter + gitoxide" | "ripgrep + gitoxide + git CLI; ast-grep and tree-sitter *planned, not built*" | Corrected |
| "The code daemon uses ast-grep, ripgrep, tree-sitter, and gitoxide" | "uses ripgrep and gitoxide today; the ast-grep and tree-sitter layer … is specified but not built" | Corrected |
| Code daemon does "symbol search, structural queries, git operations, worktree management" | "text search, git operations, worktree management; symbol and structural queries are specified but not yet built" | Corrected |
| — | Document-map row for `10-grounding-and-context.md` | New |

## `02-architecture.md`

| Was | Now | |
|---|---|---|
| Code-daemon tool bullet `• find_symbol` | `• list_symbols` — no `find_symbol` tool exists | Corrected |
| `/sessions/<pr-id>/init`, `/sessions/<pr-id>/ask`, `/sessions/<pr-id>/export` | `POST /v1/sessions`, `/v1/sessions/<session-id>/ask`, `POST /v1/sessions/<session-id>/export`. There is no `/init` route | Corrected |
| "CORS is dynamic: the allowlist is read on every request from the live extension-origin value" | "CORS is wildcard (`*`) and there is no allowlist" — the origin is kept for diagnostics only. The old claim also contradicted this document's own security bullet | Corrected |
| Conversation history "keyed by `(pr_url, turn_id)`" | Keyed by `turn_id`, `session_id` FK, `UNIQUE (session_id, ordinal)`; `pr_url` lives on `sessions` | Corrected |
| Worktrees at `…/worktrees/<owner>/<repo>/pr-<n>` | `…/worktrees/<sanitized-repo_id>/<sanitized-ref>`, e.g. `…/github.com_owner_repo/refs_pull_123_head` — two flattened segments | Corrected |
| Extension state: "Only the daemon URL and the bearer token" | Eleven keys: plus theme, panel geometry, per-session mute, last error/success, protocol mismatch, dismissed banners, onboarding flag | Corrected |
| Surfaces the error `"Code intelligence unavailable — restart the daemon"` | Surfaces the tool error inline; there is no canned string | Corrected |

## `03-code-daemon.md`

| Was | Now | |
|---|---|---|
| § Symbols and § Language Support presented as working | **Status: not built.** No grammar is compiled in (`has_grammar` returns `false`), no `ast_grep_core` dependency; the four tools are stubs answering `unsupported_language`. Kept because Phase C's LSP work is specified to fall back to them | Not built |
| "These languages are compiled in to `libre-cr-code` by default" (11-row grammar table) | "**Intended, not present.** None of these grammars is compiled in today; the table is the target set" | Not built |
| ast-grep "invoked as a library via `ast_grep_core` … we cache parsed ASTs … in an LRU" | Marked *intended*; neither the dependency nor the cache exists (`ast_cache_size` is dead config) | Not built |
| `detect_languages`: "extension + content heuristics" | "extension only" — an unrecognised extension yields `"Unknown"` | Corrected |
| `list_dir { …, recursive?, max_depth? }` | `{ repo_path, dir, ref? }` — single level; neither parameter exists | Corrected |
| `grep { …, ref?, … }` | `ref` removed: the schema declared it but the handler never read it, so a caller silently got working-tree results. Also notes `column` is always `1` | Corrected |
| `git_log { …, since? }` | No date filter; unknown properties are dropped silently by the validator | Corrected |
| `read_file` → "Returns file content, optionally sliced" | Adds that every line is prefixed with its 1-based number (`   38 \| …`) as a **contract** — an LLM reading an unnumbered blob has to count, and got it wrong in the field | New |
| gitoxide "for read operations (`log`, `blame`, `show`, `diff`, `ls-tree`)" | gitoxide for `log` and tree reads only; `blame`, `show`, `diff` shell out to the `git` CLI | Corrected |
| Clone cache `…/repos/<owner>/<repo>` | `…/repos/<repo_id>` where `repo_id` is `<host>/<owner>/<repo>` | Corrected |
| Worktrees `…/worktrees/<repo_id>/<name\|sanitized-ref>` | Both segments sanitised (`/` → `_`), with a real example | Corrected |
| `libre-cr-code config edit  # open config in $EDITOR` | Deleted — no `config` subcommand exists | Corrected |
| `worktrees  # list all worktrees + LRU stats` | "list worktrees (paths, refs, timestamps)" — no sizes or thresholds | Corrected |
| "Logs are local files only: `~/.local/state/libre-cr-code/log/<date>.log`. Rotated daily, kept 14 days." | "The daemon logs to **stderr only** — no log file, no rotation, no retention"; the supervisor captures the stream. `[logging] file` is accepted and ignored | **Dropped** |
| "`clone_repo` writes only inside `data_dir`. The managed cache is never outside the configured root." | "**should** write only inside `data_dir`. *Not enforced today*" — `target_dir` is used verbatim. Exposure bounded: the tool is hidden from the model | Filed |

## `04-review-daemon.md`

| Was | Now | |
|---|---|---|
| § MCP Server Surface presented as exposed on stdio and `/mcp`, with four tools | **Status: not built.** `mcp-stdio` prints "not implemented (Phase 4)"; no `/mcp` route; `[mcp_server]` is accepted-but-ignored config. Kept — it is the stated reason the loop is worth having outside the extension | Not built |
| "On shutdown: drain in-flight requests, gracefully close MCP child, flush SQLite" | **Not built** — no signal handler; a `SIGTERM` ends the process abruptly. Per-connection cancellation *is* implemented | Filed |
| "Daemon shutdown → all in-flight turns marked `cancelled`" | Marked *intended*; cross-references § Process Model | Filed |
| Auth exemptions: `POST /v1/pair` and `GET /v1/health` | Adds `GET /config-ui` | Corrected |
| `AskInit { question, selection?, verb?, mute_presentations? }` | Adds `context_turn_ids?: string[]`, and that a `Selection` carries its own `text` | New |
| `{ type: "tool_result", call_id, result_preview }` | Adds `truncated_from?`, with prose on what it means and that the turn still completes | New |
| Store diagram: "sessions, turns, notes, tool_traces, providers" | "sessions, turns, tool_traces, turns_fts" — there is no `notes` or `providers` table; a note is a `turns` row | Corrected |
| Schema block | Adds the two shipped columns it omitted: `sessions.head_sha` (drives the diff-changed banner, and appears in a response this same spec documents) and `turns.source_turn_id` | Corrected |
| "Bounded at `MAX_TOOL_TURNS = 25`" | Bounded by `[limits] max_tool_turns` — a config field, not a constant; the pseudo-code names are shorthand | Corrected |
| `get_pr_diff {}` → `{ files: [{ path, status, additions, deletions, hunks }] }`, "as scraped from the browser" | `{ paths?: string[] }` → `{ files: [{ path, status, hunks }] }`, computed as a three-dot `git_diff` on the worktree; the scraped payload is only the fallback | Corrected |
| `get_pr_metadata` → `{ …, files_changed }` | Five keys; no `files_changed` | Corrected |
| `stream(&self, messages: &[Message], tools: &[Tool])` | `tools: &[ToolSchema]` — no `Tool` type exists | Corrected |
| `kind = "anthropic"  # "mock" \| "anthropic" \| "openai_compat"` | Adds "default is `mock`" | Corrected |
| `[limits]` = three keys | Adds the six context caps with comments, and a paragraph on why they are tunable rather than constants (measured: 589,499 chars in one `get_pr_diff`) | New |
| Error envelope example carried `"recoverable": true` | Removed, with prose: `recoverable` is never populated on an HTTP response; only the WS `error` frame carries a real one | Corrected |
| "The extension does **not** edit provider config or store the API key" | Widened to "does **not** edit daemon config — not the provider, not the API key, and not the `[limits]` caps", and the config page's role stated after the page is introduced | Corrected |
| § Agent Loop | Adds a pointer to `10-grounding-and-context.md` for what the loop hands the model, plus the two consequences visible in the loop (hidden worktree tools; three-dot diff) | New |

## `05-browser-extension.md`

| Was | Now | |
|---|---|---|
| `daemon.token` "Stored encrypted with the extension's own obfuscation" | "**Plaintext** in `browser.storage.local`" — not obfuscated, not encrypted, and not planned to be, with the reasoning. **This was a false security claim** | **Dropped** |
| "Tailwind in Shadow DOM via WXT's CSS injection mode" | No Tailwind and no CSS framework: a hand-written CSS string in the shadow root, plus a page-level sheet via `adoptedStyleSheets` | Corrected |
| `"host_permissions": ["*://github.com/*"]` and "does not include `127.0.0.1`" | Includes loopback, and daemon calls **are** relayed through the background worker — both forced by the page-CSP discovery. The old text contradicted this spec's own § Transport | Corrected |
| `"options_page": "options.html"` | `"options_ui": { "page": …, "open_in_tab": true }` | Corrected |
| Symbol selection via "the extension's tree-sitter-lite layer (a minimal TS port)" | `pickIdentifier` is a regex over the clicked line, column from the mouse offset. No tree-sitter layer exists | Corrected |
| Deep-links `(?endpoint=…&code=…)` | `#pair?endpoint=<url>&code=<code>[&auto=1]` — a **hash**, requiring the `pair` prefix; `auto=1` gates auto-completion. A link built from the old text is ignored | Corrected |
| § Diff Interaction Layer: four affordances | Only presentation effects are built. "show on diff", reference popovers and the hover-gutter Ask button marked *not built*; `SelectionLayer` installs one click listener and renders nothing | Not built |
| § Error Surfaces table | **Status: aspirational.** No toolbar pill, retry button, elapsed estimate or "report mismatch" link; every branch renders plain text | Not built |
| Options page: presentation settings, per-PR panel reset | Marked *not built* — the page ships Pairing, Theme, Diagnostics. The two `open_link` flags are hardcoded with no UI; `autoClearOnNewQuestion` is never read | Not built |
| Storage table | Adds the four undocumented keys (`ui.last_daemon_error`, `ui.last_daemon_ok_at`, `ui.diff_change_dismissed`, `onboarding.first_pair_seen`) | Corrected |
| § Q&A Panel behaviour list | Adds markdown answers, copy-as-markdown, the truncation notice, history restore, hotkey isolation, resizable/re-openable panel | New |

## `06-investigation-verbs.md`

| Was | Now | |
|---|---|---|
| `Verb { …, output_shape: OutputShape, … }` | Field removed — it does not exist. The per-verb "Output shape" lines are prose guidance in each `system_prompt`, and nothing validates an answer's form | Corrected |
| `show_history` / `compare_to_base`: "Required selection: range or file" | "range (a symbol selection also satisfies it; a whole-file selection does not)" — a file selection leaves the button disabled | Corrected |
| § Base System Prompt given as verbatim text | **Status: illustrative.** The real prompt is assembled by `build_system_prompt` in a different order; grounding rules live in spec 10 | Not built |
| Suggested tools "determine whether the verb is **available** … disabled with an explanatory tooltip" | Availability is computed from the selection alone; nothing consults code-daemon connectivity, and a disabled verb shows its ordinary description | Corrected |
| Symbol-picker fallback: "Treat the selected line's first identifier as the symbol — proceed?" | No such prompt; an unresolved click emits no selection and symbol verbs stay disabled | Corrected |
| "Add a `Verb` struct entry in `verbs.rs`" | `verbs/mod.rs` | Corrected |
| "Export … uses verb labels as section headers" | It does **not** — export groups by severity and renders each investigation under its question. `verb` is stored but never read by the exporter | Corrected |

## `07-conversation-and-notes.md`

| Was | Now | |
|---|---|---|
| "Severity is selected from a small picker that appears when you click 'Add note'" | "Add note" posts immediately with no severity step, defaulting to `info`. The picker is what *Save as note* and note-edit offer | Corrected |
| `add_note` creates a note with "a small marker that it was agent-created" | Marked intended but **not built** — no provenance column, so an agent note is indistinguishable from a user note | Corrected |
| Export headings `## ⚠ Warning` (×2) | `## Warning` — `group_heading()` returns plain text. (The ⚠ in the *panel* mockup is real; the panel does render a glyph) | Corrected |
| Verbose export nests `<details><summary>Investigation</summary>` under each note | Notes group by severity first; all Q&A follows in one trailing `## Investigation context` section. Notes and investigations are never interleaved | Corrected |
| `GET /v1/search` → a bare JSON array | `→ { "results": [ … ] }` — an object wrapper | Corrected |

## `08-distribution.md`

| Was | Now | |
|---|---|---|
| § What Ships / Install Paths / Updates / Release Pipeline as shipped | **Status: no packaging artifact exists in this repository** — no formula, `install.sh`, service unit or Scoop manifest; `release.yml` is a Phase 0 stub. Today: `cargo build --release` from the checkout | Not built |
| `libre-cr update` with a `Current:/Latest:/Apply?` transcript and an implementation list | **Status: not built** — prints "auto-update is not implemented yet"; no version check, no signature verification, no swap. Nothing phones home either | Not built |
| "The wrapper is ~500 lines of Rust that calls into the same crate the daemons are built from" | ~2,000 lines; depends on `libre-cr-common` only and spawns the daemons **by name from `PATH`** — which is why a stale copy earlier on `PATH` shadows a fresh build | Corrected |
| First-run banner "on http://127.0.0.1:7841" | An illustrative port, plus a note that the default is **ephemeral** (`port = 0`) and the endpoint file is the authority | Corrected |
| Pairing: click "Pair with daemon", then "Run `libre-cr pair` and paste the code that appears" | Inverted — `libre-cr pair` mints the code first, then you enter it in the options page | Corrected |
| `doctor  # Diagnose: ports, file perms, code-daemon health` | "git, binaries on PATH, file perms, endpoint format" — no port check, no code-daemon probe | Corrected |
| `start [--autostart]` | Notes `--autostart` only prints a notice today | Corrected |
| Logs "(rolling, daily, 14 days retained)" | The supervisor's append-only capture of stderr; **no rotation and no retention**, so they grow unbounded. Not planned | **Dropped** |
| § Supervision Model | Adds the two-pid-file model: `supervisor.pid` is the install, `review.pid` is a child whose pid changes per restart; why `stop` must target the supervisor; orphan reaping; `status` flagging `running unsupervised` | New |
| "equivalent locations on Windows (`%APPDATA%`, `%LOCALAPPDATA%`)" | Windows is **not** special-cased: the helpers are `$XDG_*`-or-`$HOME` everywhere, so Windows lands in `%USERPROFILE%\.config\libre-cr`. Also adds the omitted `install_key` file | Corrected |
| Uninstall: removes binaries, separate keep-data / keep-logs prompts, `~/.local/share/libre-cr-*` | One all-or-nothing confirmation; removes `~/.local/share/libre-cr`, which is **not** the glob — the real databases in `libre-cr-review/` and `libre-cr-code/` survive; never removes binaries | Corrected |
| "Token is regenerated on `libre-cr restart --rotate-token`" | Marked *intended*; `restart` takes no flags and nothing rotates a token. Manual equivalent: delete the token file and restart | Not built |
| "Daemon refuses to start with a config file with mode wider than 0644" | "**should** refuse … **Not enforced:** neither daemon inspects config permissions." `doctor` does check the token and endpoint modes | Filed |

## `09-presentation-tools.md`

| Was | Now | |
|---|---|---|
| `highlight_lines(file, start_line, end_line, color?, label?)`; "`label` is a tooltip" | `(file, start_line, end_line, label, detail, color?)` — both **required** in the schema. `label` is a caption chip, `detail` one to three self-contained sentences shown in the tour | Corrected |
| — | **A span is capped at 80 lines**, with the clamp reported to the model. A 479-line span once killed the tab; resolution is now one DOM scan per call | New |
| `open_link` `target="panel"` "opens it in a small embedded iframe panel … when the URL is from a known-safe origin" | **Not built** — no iframe exists. Documents what the URL check really allows: `https://` anywhere, `http://` on `127.0.0.1`, and any root-relative path with no GitHub-origin check | Not built |
| Success envelope `"result": { "applied": true, "effect_id": "h_xyz" }` | `{ "effect_id": "e_7" }` — no `applied` field; ids are `e_N`. Adds `note`, the channel a clamped highlight uses | Corrected |
| "Effects are also namespaced by `turn_id` so we can scope clearing" | The `turn_id` is optional and never assigned; there is no `session_id`. Clearing is scoped by tag, not turn | Corrected |
| "Tracks every effect by `(turn_id, effect_id)`" | By `effect_id` and tool; per-turn keying marked intended | Corrected |
| "auto-clear when the next question is asked, manual override available. Effects from notes … are not cleared automatically" | Clearing is **unconditional**; `autoClearOnNewQuestion` is never read. Notes place no DOM effects at all — the only tags are `highlight`, `annotation`, `flash`, all agent-placed | Corrected |
| "the existing `UIController` implementations from the POC … The POC code stays in place" | No `UIController` type survives; the handlers were rewritten. What changed is who calls them | Corrected |
| `enum ToolBackend { CodeDaemon, Internal, Presentation }` | Marked illustrative; the shipped router uses `enum Category { Internal, CodeDaemon, Presentation, Unknown }`, and only `PresentationDispatcher` is real | Corrected |
| `related_tests` hint: "scroll to the first cited test file … `annotate_line` only if there's a specific concern" | *Not implemented as a hint* — that prompt carries no such instruction, and `annotate_line` is in no verb's suggested tools. The other four hints are real | Corrected |
| § Extension Implementation | Adds how effects survive a page we do not own: attribute-keyed marking (React rewrites `className`), `adoptedStyleSheets` (CSP), `ensureFileRendered` for the virtualized diff, and why clearing strips highlights but removes annotation rows | New |
| § User Controls | Adds the guided tour: recorded steps, Prev/Next, label+detail beside the code, armed-open, and that **scrolling only ever follows a reviewer action**. Replaced a timed replay whose pacing was never right | New |

## `10-grounding-and-context.md` — new

288 lines, no predecessor. The contract for what evidence reaches the model,
how much, and what is reported when it is cut: grounding rules and the failure
each came from; evidence on the wire (`Selection.text`, numbered reads, the
three-dot diff, worktree tools hidden); the three history-replay tiers and
`context_turn_ids`; the context budget with its measured numbers and the rule
that exceeding a cap never fails a turn; reporting to the model and to the
reviewer; a failure-mode table; an explicit "what this does not do"; and a
provenance table mapping every rule to its ledger entry.

Written **after** the code — see the laundering caveat above. Its normative
claims are decisions that could have gone otherwise; its recitation of cap
values duplicates `[limits]` and should become a pointer.

## `CHANGELOG-TESTING.md`

| Was | Now | |
|---|---|---|
| — | Entry: "The model explained the wrong line" — the selection-text and numbered-reads fix shipped in `e4f8c96` but only its replay half had ever been recorded. Found because a provenance citation to it did not resolve | New |
| — | Entry: "A wide `highlight_lines` froze (and killed) the tab" | New |
| — | Header note: this log records what changed and why; the resulting contract is in spec 10 | New |
| "`get_pr_diff` is computed by the router via `git_diff origin/<base>..HEAD`" | Three-dot (`merge_base: true`), with why two-dot — the notation the fix rejected — is wrong | Corrected |
| "the footer button renamed 'Clear highlights'" | Records both names: renamed again to "Clear all effects" in the CodeRabbit round, because it also clears annotations and flashes | Corrected |
| "all remain marked planned in `08-distribution.md` / `plan.md`" | `plan.md` marks them planned; **`08-distribution.md` does not** — it presented several as shipped. Same fabricated-cross-reference class as the one caught by hand | Corrected |
| Four `§ Section` references naming no real heading | All corrected. A fifth, wrapped across two lines, was found by a checker that parses every heading in every spec — the audit had missed it. 0 unresolved | Corrected |
| "BUG — `libre-cr stop` does not stop the supervisor" (open) | Fixed; the entry now carries only the remaining half (the supervisor still runs in the foreground) | — |
| "Wrapper SIGKILL can leave a live unsupervised review daemon" (open) | Removed — fixed by the same change | — |
| "Log rotation is still a TODO" (open) | "**Log rotation: decided against**", with the reasoning | **Dropped** |
| — | Backlog: no graceful shutdown in the review daemon; `clone_repo` containment and config-mode checks unenforced | Filed |

---

## What this record does not cover

- **Nothing was verified by running the system.** The audit and these
  corrections are static: spec text read against source. Two spec sections
  remain unverifiable this way — § Performance Budget (03) and § Performance
  Targets (05) are runtime measurements with no constants to check.
- **The `plan.md` phases were not re-audited.** Where a section is marked
  "still on the plan", that reflects `plan.md` as written, not a fresh
  judgement about whether the phase will happen.
- **Correctness of the code was not assessed.** Every "Corrected" row above
  means the spec disagreed with the code and the code was taken as right. That
  is the correct default for a description, and exactly the wrong default for a
  specification — which is the distinction this document opens with.
