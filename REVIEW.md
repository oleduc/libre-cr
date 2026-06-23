# Spec Review Guide

A reviewer's companion to the spec set under `specs/`. One section per spec file. Each section calls out:

- **The central decision** the file makes. If this is wrong, the file is wrong.
- **Contestable secondary decisions.** Defensible but worth pressure-testing.
- **Open items / risks.** Where I (or whoever drafted) am least sure.
- **Cross-refs.** Where the file's claims meet other specs.

This document is not a recap. Read the specs themselves; use this to focus the review and surface what to push back on.

---

## Meta — cross-cutting decisions worth confirming

These show up in multiple specs. If any of them break, the whole shape changes.

1. **Human-driven Q&A, not LLM-orchestrated review.** The product abandons the POC's autonomous model. *Pressure test:* the autonomous players (CodeRabbit, Copilot, Greptile) are well-funded; we're betting the open-source human-driven niche is real demand, not just our preference. Do user interviews / a competitor scan validate this before sinking 4–6 months?

2. **Two daemons, single browser extension, MCP between.** A code daemon worth shipping standalone (`libre-cr-code`) plus a product-specific review daemon (`libre-cr-review`). *Pressure test:* the standalone code daemon overlaps with Serena and mcp-language-server. Bet is "Rust-native, no-Python-dep, PR-tuned." Does that bet hold up?

3. **Rust for daemons, single static binary distribution.** *Pressure test:* implies 4–6 weeks before any user-visible product behavior. A faster language + bundling approach could shorten the loop. Worth the patience?

4. **BYOK + local-only state, no server.** No telemetry, no auth backend, no team sync. *Pressure test:* limits future monetization paths (you can't run a SaaS on this); also limits team workflows. Intentional and fine, but flag it.

5. **Phase B = AST-based code intelligence; Phase C = LSP.** Ships imperfect cross-file resolution first. *Pressure test:* if find_references returns half-noise on real codebases, the product feels broken. Do we trust the `confidence` field to keep us honest, or should LSP be in Phase B for at least one language (Rust → rust-analyzer)?

6. **Investigation verbs (5) + free-form question.** *Pressure test:* the right starter set? Hard-coded verbs vs. user-customizable verbs?

7. **Browser extension is thin; daemon owns everything stateful.** *Pressure test:* the pairing flow (extension reads endpoint file / accepts deep link) is the friction-heaviest UX touchpoint in the product. Is it good enough?

---

## `01-overview.md` — product thesis

**Central decision:** Replaces the POC's LLM-as-orchestrator with human-driven Q&A on a local daemon backend.

**Contestable secondary decisions:**
- **No GitHub OAuth.** Posting reviews stays clipboard-driven in v2. Could feel primitive next to CodeRabbit's inline comments.
- **No autonomous review at all.** A "scan for concerns" verb is recorded as a future possibility but explicitly not in Phase B. Some reviewers actually want that; we're betting the human-driven primary loop is enough.
- **LLM provider is BYOK.** No bundled Anthropic key or trial. Means zero-friction onboarding for someone "just trying it" is impossible — they need an API key first.

**Open items / risks:**
- The "what carries over from POC" list assumes the GitHub adapter survives current markup; that was true a few weeks ago, GitHub's React migration is ongoing. Worth re-verifying before Phase 5.
- The non-goals list is honest but aggressive. "No code editing or rewriting" is a real constraint — if early users want "fix this for me," we'll feel the pull to add it.

**Cross-refs:** every spec ultimately derives from this file's principles.

---

## `02-architecture.md` — system shape

**Central decision:** Three components in a strict ownership hierarchy: code daemon (repo intelligence, no PR awareness), review daemon (PR awareness, agent loop, conversation state), browser extension (selection + render only).

**Contestable secondary decisions:**
- **Localhost HTTP + WebSocket + bearer token** for extension↔daemon transport, rejecting Native Messaging. Trades simpler debugging + multi-client support for the bearer-token pairing dance.
- **Review daemon spawns code daemon as child by default,** with an "external daemon" power-user mode. Couples lifetimes; a code daemon crash takes the review daemon's tool calls down until restart.
- **API key lives only in the review daemon's config, never in the extension.** This is the right boundary but means we ship a config web UI on the daemon's port — another moving piece.
- **`presentation_call` / `presentation_result` frames** on the same WS used for streaming answers. Multiplexing means we need careful cancellation semantics.

**Open items / risks:**
- The dataflow assumes the extension can do `fetch` to `127.0.0.1` from a content script. Browsers' treatment of localhost from content scripts is in flux; we may end up routing through the background service worker after all. Documented as fallback but worth verifying early.
- The state-locations table grew organically — verify each row maps to exactly one owner.

**Cross-refs:** every other spec.

---

## `03-code-daemon.md` — `libre-cr-code`

**Central decision:** A Rust MCP server that owns repo discovery, worktree management, and code intelligence. Stateless tools (take `repo_path` per call). Phase B uses ast-grep + ripgrep + tree-sitter + gitoxide.

**Contestable secondary decisions:**
- **Per-call `repo_path` rather than connection-scoped.** We considered the alternative (Option E hybrid) and rejected it as premature complexity. *Worth re-evaluating once we have real external MCP clients using the daemon.*
- **Phase B language set: 11 grammars** (Rust, Go, JS, TS, Python, Java, C, C++, Ruby, PHP, Bash). Fast-follow: C#, Kotlin, Swift, Scala. *Right cut?*
- **SQLite for the repo registry.** Adds a state file the daemon must manage. Could be a flat JSON for v2; would simplify "first run on a new machine."
- **Worktree LRU eviction with 5 GB default.** Number is a guess.
- **`confidence` field on `find_definition` / `find_references`** to telegraph name-based-not-semantic. Relies on the LLM honoring it.

**Open items / risks:**
- **Phase B's name-based AST might be too noisy in practice.** This is the single biggest "will this product feel good" risk in the spec. Some experiments on real polyglot repos before committing would be cheap insurance.
- The "tools/list lists 20+ tools" claim hasn't been pressure-tested against MCP client UIs that struggle with large tool sets.
- `git_diff` returns structured hunks; do we trust gitoxide's hunk parsing for all the corner cases (binary files, renames, mode changes)?

**Cross-refs:** `04-review-daemon.md` (the consumer); `plan.md` Phase 1 (build sequence); `09-presentation-tools.md` (separate tool category, doesn't touch this).

---

## `04-review-daemon.md` — `libre-cr-review`

**Central decision:** Hosts the LLM client + agent loop, owns per-PR conversation state in SQLite, orchestrates the code daemon over MCP, serves the extension over HTTP/WS, and exposes a high-level MCP server for external clients.

**Contestable secondary decisions:**
- **Single in-flight ask per session** (returns 409). Simpler than queueing; user-visible cost is "wait for the last question to finish."
- **`MAX_TOOL_TURNS = 25`.** Hand-picked.
- **AES-GCM with machine-bound key for the stored API key.** Reasonable but not strong against a determined local attacker. Could justify a passphrase-prompt mode later.
- **Verb registry hardcoded in Rust source.** Plugin model explicitly deferred. Good for v2; will feel restrictive once users want their own verbs.
- **Conversation history stored as raw turns + tool_traces, with FTS.** The traces are large; bytes per session could exceed estimates.
- **External MCP surface narrow:** only `ask_about_pr`, `list_sessions`, `get_session_history`, `export_session`. No raw tool exposure here.

**Open items / risks:**
- **Tool router complexity.** The router merges code-daemon tools (dynamic), internal tools (static), and presentation tools (only when there's an active WS). Three categories with three dispatchers; the test surface is bigger than it looks.
- **The encryption-key-derivation story.** "Machine-bound material" needs a real definition. Likely a per-install random key stored at install time, not derived from the machine ID.
- **Session lifetime:** "evict after 90 days idle" is arbitrary.
- **Cancellation semantics during a multi-tool turn.** Spelled out at a high level; the corner cases (LLM call streaming + parallel tool dispatches + WS disconnect mid-flight) need careful implementation.

**Cross-refs:** `03-code-daemon.md` (downstream); `09-presentation-tools.md` (third tool category); `06-investigation-verbs.md` (verb registry); `07-conversation-and-notes.md` (storage model).

---

## `05-browser-extension.md` — the extension

**Central decision:** A thin extension that scrapes PR pages, captures selections, opens a Q&A panel, and renders streamed answers + presentation effects. Carries forward the POC's GitHub adapter, Shadow DOM shell, and `UIController` machinery.

**Contestable secondary decisions:**
- **No conversation persistence in the extension.** Daemon is the source of truth. Means "daemon down → no history visible" — even past sessions disappear from the popup.
- **Pairing flow via endpoint file + token + optional deep link.** Awkward but documented. Manual paste fallback is the safety net.
- **Symbol selection via tree-sitter-lite or regex fallback.** Real symbol resolution lives in the daemon; the extension just needs "what's the identifier at the cursor?"
- **No background service worker proxy** by default — content script talks directly to localhost. Fallback exists if CORS gets weird.

**Open items / risks:**
- **First-run UX is the most user-hostile part of the product.** Install extension + install daemon via brew + run daemon + pair via code. Five steps. Compare to "install CodeRabbit GitHub App: one click."
- **GitHub selector resilience** is a continuous maintenance burden we're inheriting from the POC. The spec doesn't yet define the "selector mismatch" reporting/repair workflow.
- The diff-interaction layer (line highlights, gutter affordances, presentation effects) shares DOM space with GitHub's own React tree. Reconciliation collisions are the most common breakage mode in this category of extension.

**Cross-refs:** `02-architecture.md` (transport details); `09-presentation-tools.md` (which the extension executes); `04-review-daemon.md` (the API contract).

---

## `06-investigation-verbs.md` — verb catalog

**Central decision:** Five named verbs (`find_callers`, `show_history`, `related_tests`, `compare_to_base`, `explain`) + free-form, each a prompt-tuned shortcut over the same agent loop.

**Contestable secondary decisions:**
- **Verbs are hardcoded** in the review daemon's source for v2. Adding one = code change + spec update + version bump.
- **Required-selection enforcement is strict** — symbol verbs gray out if the selection isn't symbol-shaped. Could be more forgiving (best-effort resolution).
- **No "check for concerns" verb.** Recorded as a future option; deliberately excluded because it edges back toward autonomous review.
- **Each verb's prompt addendum is hand-written.** No template; just prose. Has to be tuned against real PRs, not theory.

**Open items / risks:**
- **Are these the right 5?** The honest answer is "we'll find out." `find_callers` is obviously valuable; `compare_to_base` is borderline (the diff already shows this). `related_tests` might be the highest-leverage one and is also the hardest to get right (heuristics on test file paths break on monorepos).
- **Prompt fragility.** A verb's quality is its prompt's quality. We need a prompt-eval harness in CI before users see it.
- The cross-verb behavior when one is run after another in the same session is unspecified. Does context carry?

**Cross-refs:** `04-review-daemon.md` (verb registry mechanism); `09-presentation-tools.md` (per-verb presentation hints).

---

## `07-conversation-and-notes.md` — persistence + export

**Central decision:** Conversation turns and user notes are both stored as rows in `turns` with a `kind` discriminator. Export assembles notes (not turns) into a Markdown draft for clipboard paste.

**Contestable secondary decisions:**
- **Q&A turns are immutable; notes are editable.** Keeps history honest; lightly opinionated.
- **"Save as note" turns assistant answers into editable notes,** creating a new note row with a back-reference to the source turn.
- **Export defaults to notes-only.** Verbose mode adds investigation context. Full transcript is opt-in.
- **GitHub posting deferred to a later phase.** Clipboard works; OAuth adds support burden.
- **No edit history on notes.** Edit overwrites.

**Open items / risks:**
- **The Markdown-to-GitHub-review composer paste-flow** is friction. The "inline comment drafts the user pastes one by one" path is documented honestly but is ugly UX; users will want one-click posting fast.
- **Severity model duplicates GitHub's own.** We assign severities; GitHub's review interface has its own concepts. Conversion is documented but not verified against the GitHub composer's actual behavior.
- **Notes containing diff snippets** could include code that's sensitive (secrets in the diff, internal endpoints). Documented as a known leak path; not mitigated.

**Cross-refs:** `04-review-daemon.md` (SQLite schema); `05-browser-extension.md` (note creation UI).

---

## `08-distribution.md` — install + supervision

**Central decision:** Three binaries (`libre-cr`, `libre-cr-review`, `libre-cr-code`) + extension. Wrapper CLI (`libre-cr`) supervises both daemons. Brew + Scoop + GitHub Releases.

**Contestable secondary decisions:**
- **Two-layer supervision** (wrapper supervises review daemon; review daemon supervises code daemon). Robust but more moving parts.
- **No autostart by default.** User runs `libre-cr start` after install. Tradeoff: lower surprise, higher friction.
- **Manual update flow.** `libre-cr update` checks a signed manifest. Auto-update is a future opt-in.
- **No bundled language servers** (Phase C only — and users install LSPs themselves).
- **AES-GCM + machine-bound key for API key storage.** Same caveat as in `04-review-daemon.md`.

**Open items / risks:**
- **The wrapper CLI is documented as "~500 lines of Rust"** but its actual surface (start/stop/restart/status/logs/pair/doctor/update/uninstall) is non-trivial. Underestimated.
- **Signed binaries imply** an Apple developer account, an Authenticode cert, and a GPG keyring management story. Real ops work, not free.
- **`libre-cr-code` ships its own brew formula** to support standalone use. That's two separate brew formulas to maintain on each release.

**Cross-refs:** `02-architecture.md` (process model); `04-review-daemon.md` (config layout); `plan.md` (Phase 7).

---

## `09-presentation-tools.md` — LLM-dispatched browser actions

**Central decision:** A fixed set of 5 tools (`highlight_lines`, `annotate_line`, `scroll_to`, `open_link`, `clear_presentation`) the LLM can call during a turn. Routed back through the WS to the extension. The POC's `UIController` powers them.

**Contestable secondary decisions:**
- **Bounded set, hardcoded.** No user/function-pack extension. Easy to audit; constraining.
- **Auto-clear on next question by default,** override available. Right default for our user (a working reviewer); might surprise users who scroll through history.
- **`open_link` URL validation is strict** (https or known-safe). Limits agent's ability to surface, e.g., docs links from non-https sources.
- **External MCP surface (`ask_about_pr`) does not register presentation tools.** Cleanly modeled (no WS = no presentation), but means external clients can't get presentation effects in a separate UI.

**Open items / risks:**
- **The agent's prompt has to discourage overuse.** History will show LLMs love to highlight things. The base prompt's discouragement is honest; calibration on real models matters.
- **`annotate_line` recreates the POC's failure mode at smaller scale.** If the LLM annotates 20 lines per turn, we've reinvented the thing we abandoned. The "sparing-by-prompt" rule is the only guardrail.
- **Effect lifecycle on page navigation** (e.g., user navigates between PR files): documented as "clear on content script invalidation." Practical behavior depends on GitHub's Turbo lifecycle, which is fiddly.

**Cross-refs:** `04-review-daemon.md` (tool router; WS frames); `05-browser-extension.md` (presentation handler); `06-investigation-verbs.md` (per-verb usage hints).

---

## `plan.md` — phased implementation

**Central decision:** Ten phases (0–9 + C). Phase 0 scaffolding, 1+2 parallel (code daemon, review daemon skeleton), 3 wiring, 4 verbs, 5 extension + first demo, 6 export + polish, 7 distribution, 8 hardening, 9 optional OAuth, C LSP track.

**Contestable secondary decisions:**
- **Phases 1 and 2 parallelizable** assumes the shared-types crate (`libre-cr-common`) is settled early. Realistic but it's a coordination point.
- **First demo milestone is at Phase 5.** That's roughly 4–6 weeks of work before anything is user-visible. Painful but the alternative (visible UI on a fake backend) trains us to demo lies.
- **Phase 9 (GitHub OAuth) is optional and post-v2.** Defensible; will be the most-requested feature once v2 ships.
- **Phase C (LSP) is parallel-track, not sequential.** Can develop against Phase 1 alone. Good — but staffing-wise it competes with Phases 5–7.

**Open items / risks:**
- **The phase sizes ([S], [M], [L]) are estimates without calibration.** Phase 1 [M] for the code daemon is probably [L] honestly — 11 grammars + 4 search backends + worktree management is real work.
- **No prompt-evaluation phase.** Prompt tuning is embedded in verbs (Phase 4) and presentation tools (Phase 5). It deserves its own dedicated activity with a real harness.
- **No security review phase.** The spec touches API key storage, localhost auth, URL validation, code daemon's filesystem reach — worth a dedicated pass.
- **No external-user feedback gate** between Phase 5 (demo) and Phase 6 (first usable). We could ship Phase 5 to 3–5 trusted reviewers and learn before continuing.

**Cross-refs:** every other spec — this is the build sequence.

---

## Cross-cutting questions not assigned to any single file

Things worth deciding before serious implementation begins:

1. **Validation: does v2 solve a problem people actually have?** Concrete next step: 3–5 30-minute interviews with target users (engineers who regularly review PRs and find AI tools insufficient). Spec assumes the human-driven niche is real; not yet validated.

2. **Competitor scan completeness.** Earlier discussion identified the autonomous players + Serena/mcp-language-server + IDE agents. Has anyone scanned MCP server registries, Hacker News / Lobsters, and GitHub topic searches in the last week? 1–2 hours of due diligence is cheap.

3. **Time-to-first-answer in the demo.** Right now the spec implies: install (5–10 min) + pair (1 min) + worktree fetch (10 s – minutes depending on repo) + first answer (~5 s). The fetch is the variable. For a fresh repo we'd be `git clone`ing — on a large repo, multiple minutes. Should the first answer use the extension-scraped data while the worktree warms up?

4. **Project name.** "libre-cr" is fine internally but soft. If we're going to publish brew formulas, register a domain, etc., is this the name we want? Names are cheap to change pre-Phase 0, expensive after.

5. **License posture.** MIT carries over from POC. Fine. But "permissive open source for a dual-daemon agent that calls user-configured LLMs" — any clauses worth thinking about (e.g., explicit no-warranty around LLM output)?

6. **CI cost.** Cross-compiling Rust to 5 targets + tree-sitter grammars + signed releases isn't free. GitHub Actions minutes for a workflow that runs on every PR will add up.

7. **Prompt versioning.** When we change `find_callers`'s system prompt, do old conversation exports get the old prompt's voice? Is the system prompt stored alongside the turn for replay fidelity?

---

## How to use this document

- **Skim the meta section first.** If those decisions look wrong, the rest of the review is faster — many spec details will need revisiting.
- **For each file: read the central decision, then the open items.** Don't re-read the spec; trust that this document represents it.
- **Note disagreements with line references** (e.g., "disagree with `02-architecture.md:89` on …") so changes can be targeted.
- **Open the cross-cutting questions at the end of the review,** since some require decisions outside the spec (interviews, name, license).
