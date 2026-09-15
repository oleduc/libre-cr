# Presentation Tools

## Purpose

A small set of tools the LLM can call to *demonstrate* its answer in the user's browser — highlight a line being cited, annotate a finding, scroll to a referenced location, open a link to a related PR. Distinct from code-intelligence tools (which gather information) and internal tools (which read session state).

These are deliberately a secondary capability. The product is human-driven: the user asks, the LLM answers in text. Presentation tools amplify that answer when they help the reviewer follow it; they do not replace the answer.

## Distinction From The POC's "LLM-As-Orchestrator" Model

The failed POC let the LLM drive the entire experience — it autonomously annotated every line it found suspicious, before the user asked anything. That was the wrong product.

Presentation tools in v2 are different along three axes:

| | POC (failed) | v2 (presentation tools) |
|---|---|---|
| Initiation | LLM decides what to highlight, with no user prompt | User asks a question; presentation calls come *in service of the answer* |
| Scope | Whole-diff annotation pass | Bounded to the current turn; cleared at user discretion |
| Salience | Always-on, persistent | Visible while the turn is "active"; user clears between turns or globally |
| Text role | Annotations were the output | Answer text is the output; presentation is supplemental |

The mechanism (DOM injection, tagged effects, the POC's highlight/annotate/scroll code) carries over. The product semantics are different.

## Tool Catalog (Phase B)

Five tools. Conservative on purpose — we add more after watching real use.

### `highlight_lines`

```
highlight_lines(file, start_line, end_line, label, detail, color?: enum)
```

Visually highlight a line or range in the diff. `color` defaults to a neutral blue; the agent can pick from `red`, `yellow`, `green`, `blue`, `purple` for cue value.

`label` and `detail` are **required**, in the schema as well as in the description. `label` is the short heading of the part (≤ 8 words), rendered as a caption chip at the right end of the range's first code line; `detail` is one to three plain sentences explaining that part, shown beside the code in the tour widget. `detail` has to stand on its own, because the reviewer reads it away from the chat.

**A span is capped at 80 lines.** Over that, the head of the range is highlighted and the model is told it was clamped, in the `presentation_result`. This is a safety belt, not the mechanism: the tool description tells the model to make several narrow highlights instead of one sweeping range, because a highlight is meant to mark *the lines an answer talks about*. Asked to mark "the DynamoDB layer", a model once requested a 479-line span; each line was resolved by its own document-wide DOM query, and the tab died. Resolution is now one scan per call regardless of span, and the cap bounds the row mutations.

Use when: the agent's answer cites a specific range and the reviewer would benefit from seeing it.

### `annotate_line`

```
annotate_line(file, line, summary, detail?: string, severity?: enum)
```

Insert an inline annotation card next to a diff line. Same row-injection technique as the POC. Severity drives styling (`info`, `suggestion`, `warning`, `critical`).

Use when: the agent has a *specific* concern about a *specific* line. Not for general commentary — that goes in the answer text or as a note.

### `scroll_to`

```
scroll_to(file, line?: number)
```

Scroll the diff view to a file (and optionally a line) and apply a brief flash highlight so the reviewer's eye lands on it. Idempotent — calling it twice in the same turn just re-flashes.

Use when: walking the reviewer through a sequence ("first this", "then that"), or when an answer references a location the reviewer should look at to verify the claim.

### `open_link`

```
open_link(url, target?: "tab" | "panel")
```

Open a URL. `target="tab"` opens in a new browser tab (default).

`target="panel"` is **not built** — no embedded iframe exists anywhere in the extension, and the panel target is refused unless a context flag that has no UI is set. What the URL check actually allows: `https://` anywhere, `http://` on `127.0.0.1` only, and any root-relative path (`/owner/repo/pull/1`) — the last with no verification that the page is on GitHub. Tighter origin checking is intended, not built.

Use when: the answer references another GitHub PR or issue, a commit URL, external documentation, etc. The LLM should not invent URLs — only use ones it discovered via tools.

Safety: the extension URL-validates before opening. URLs must be `https://` or be GitHub-hosted relative paths. `javascript:`, `data:`, and other schemes are rejected.

### `clear_presentation`

```
clear_presentation(scope?: "all" | "highlights" | "annotations")
```

The agent clears its own previously-placed presentation effects. Useful when explaining a sequence: highlight A, talk about A, clear, highlight B, etc.

Note: the user can also clear at any time via a button in the Q&A panel; this tool is for the agent to be a tidy citizen of the diff.

## Protocol

Presentation tools live in the review daemon's tool router as a third category. The router knows that calls to these names need to be routed *back* through the WebSocket to the extension for execution, rather than dispatched to the code daemon or to internal Rust functions.

### New WebSocket frames

Daemon → extension (during a turn):

```json
{ "type": "presentation_call",
  "call_id": "p_abc123",
  "tool": "highlight_lines",
  "input": { "file": "src/auth.ts", "start_line": 42, "end_line": 48, "color": "red", "label": "md5 call site" } }
```

Extension → daemon (in reply):

```json
{ "type": "presentation_result",
  "call_id": "p_abc123",
  "ok": true,
  "result": { "effect_id": "e_7" } }
```

Or on failure:

```json
{ "type": "presentation_result",
  "call_id": "p_abc123",
  "ok": false,
  "error": "file_not_in_view",
  "message": "src/auth.ts is not currently in the diff view." }
```

A successful result carries `effect_id` (ids are `e_1`, `e_2`, … per session) and, when the call was altered to fit a limit, a `note` — that is the channel a clamped `highlight_lines` uses to tell the model its range was cut to the first 80 lines. There is no `applied` field.

The daemon treats the `presentation_result` exactly like a regular tool result — feeds it back to the LLM as the tool result block. If the extension reports a failure (e.g., file not visible), the LLM sees that and can adapt (e.g., explain in text instead).

### Result envelope

Successful results always include an `effect_id` so the extension and daemon can later reference the effect (e.g., for `clear_presentation`). Effect records carry an optional `turn_id`, but nothing assigns it today and there is no `session_id` on them — clearing is scoped by tag (`highlight` / `annotation` / `flash`), not by turn. Per-turn scoping is intended, not built.

## Effect Lifecycle

```
Turn starts        → effects bucket for this turn opens
Tool call(s) apply → effects accumulate, each tagged turn_id + effect_id
Turn ends          → effects remain visible
User reads answer  → may interact with effects (click annotation, follow scroll)
User does one of:
  • Asks next question     → previous turn's effects auto-cleared if setting is on (default)
  • Clicks "Clear" on panel → all this session's effects cleared
  • Toggles "Keep effects"  → effects persist until manually cleared
  • Closes the panel        → effects cleared (restorable: see below)
  • Navigates away          → effects cleared automatically on content script unload
```

### Restoring an answer's effects

Closing the panel clears the page, and that stays: highlights left behind by a
closed panel are litter on a page the reviewer did not ask us to mark. What was
missing is the way back — reopening left an answer that talks about lines
nothing points at any more.

Every presentation call a turn made is already stored, as an ordinary tool
trace. Restoring is therefore a **replay of what was recorded**, not a cache
the extension keeps: `GET /v1/sessions/:id` carries each turn's successful
presentation calls, and the panel shows a control on an expanded answer —
"Show on diff (N)" — that applies them.

- **One answer's effects at a time.** Replaying clears whatever is currently
  shown first. Two answers' highlights on one diff cannot be told apart, and
  the tags carry no answer identity a reader could use.
- **Only successful calls replay.** A call that failed painted nothing, so
  replaying it would only fail again.
- **Replay is best-effort, and says so.** The diff may have moved since: a file
  collapsed, a line gone after a push, a range clamped. Steps that no longer
  land are skipped and the control reports what did (`3 of 5 shown`), because
  silently showing less than the answer describes is the failure this whole
  document exists to avoid.
- The control appears only on an expanded answer that recorded at least one
  call, so a conversation of plain answers gains no new furniture.

Clearing on a new question is **unconditional** today: there is no "keep effects" setting, and `autoClearOnNewQuestion` is declared but never read. Notes place no DOM effects at all — the only three tags are `highlight`, `annotation` and `flash`, all agent-placed — so there is nothing reviewer-curated to preserve. Both the setting and note-effect carve-out are intended, not built.

### Applying effects to a page we do not own

Two properties of GitHub's diff make the naive implementation fail:

- **Effects are keyed by attribute, not class.** GitHub's React re-renders diff
  rows on hover and selection and rewrites `className`, which silently erased
  highlights. Effects are therefore marked with `data-libre-cr-*` attributes —
  unknown `data-*` survives React's reconciliation — and styled through
  `adoptedStyleSheets` on attribute selectors, which is also immune to the
  page's CSP.
- **The diff is virtualized.** Files away from the viewport are placeholder
  regions with no rows, so an effect targeting one finds nothing.
  `ensureFileRendered` scrolls the placeholder (or clicks the file-tree link)
  into view and waits for the table to mount, with a deadline, before the
  effect is applied. A file that never mounts yields `file_not_in_view` rather
  than a silent no-op.

Clearing distinguishes what we inserted from what we merely marked: annotation
rows are ours and are removed, while highlight and flash markers sit on
GitHub's own rows and are only stripped. Removing a flash-tagged element once
deleted the diff row with it.

## Extension Implementation

Two roles in the extension:

1. **Presentation handler.** Listens for `presentation_call` frames on the active WS. Looks up the tool name in a registry, validates the input shape, executes via the handler functions in `utils/presentation/handlers.ts`, replies with `presentation_result`.

2. **Effect bookkeeping.** Tracks every effect by `effect_id` and tool (per-turn keying is intended, not built — see § Result envelope). Renders the Q&A panel's "X effects applied · [Clear]" footer. Implements the auto-clear logic on new-question / panel-close / nav.

The highlight/annotate/scroll implementations descend from the POC's UI-controller code, rewritten rather than kept verbatim — no `UIController` type survives in the tree. What changed is who calls them: the extension itself, via the presentation handler, in response to daemon frames.

## Daemon Implementation

Tool router gains a third dispatcher:

```rust
// Illustrative. The shipped router classifies a call by name into
// `enum Category { Internal, CodeDaemon, Presentation, Unknown }` and
// dispatches accordingly; of the types named below only
// `PresentationDispatcher` exists.
enum ToolBackend {
    CodeDaemon(McpClient),               // dispatch via MCP child
    Internal(InternalToolFn),            // call Rust fn directly
    Presentation(PresentationDispatcher) // send frame on the turn's WS, await reply
}
```

The presentation dispatcher needs access to the active WebSocket sink for the current turn — it's part of the `TurnContext`. Each `presentation_call` gets a fresh `call_id`; the dispatcher inserts a oneshot channel into a per-turn `pending_calls` map, sends the frame, and awaits the result. Timeout: 5 seconds (the extension should respond within tens of ms; 5s is generous and catches drop-the-socket cases).

The tools' input schemas are part of the daemon's tool registration. The LLM sees them just like any other tool.

## Prompt Guidance

Append to the base system prompt (see `06-investigation-verbs.md`):

```
You have presentation tools that affect what the reviewer sees in the browser:
  • highlight_lines, annotate_line, scroll_to, open_link, clear_presentation

Use them to amplify your answer when they help the reviewer follow it.
Specific guidance:

- When your answer cites a specific file:line, calling scroll_to or highlight_lines
  makes the citation directly navigable. Prefer one or the other — don't both
  scroll AND heavily highlight unless the reviewer needs both cues.

- annotate_line is for a specific concern at a specific line. Do not use it for
  general commentary, summaries, or anything that isn't actionable. The reviewer
  is in charge of their own annotations.

- open_link only for URLs you discovered via tools (e.g., a referenced PR in
  a commit message). Never construct URLs from a pattern; invented links erode
  trust.

- These tools are not a replacement for textual answers. If you find yourself
  emitting only presentation calls and no text, you are doing it wrong. Write
  the answer first; reach for presentation tools to help the reviewer trace it.

- Be sparing. Three highlights in one turn is fine; ten is noise. If you find
  yourself wanting to mark many places, mark the most important and describe
  the rest in text.

- The reviewer can clear your effects at any time. Don't take it personally.
```

The base prompt is appended after this guidance with verb-specific addenda. So every verb plus free-form has the same baseline.

### Per-verb hints (where natural)

- `find_callers`: a closing instruction to call `highlight_lines` on each cited call site (max ~5), so the reviewer can flip through them.
- `show_history`: no presentation calls by default — history is a temporal narrative, not a spatial one.
- `related_tests`: *not implemented as a hint* — this verb's prompt carries no scroll or annotate instruction, and `annotate_line` appears in no verb's suggested tools. The other four hints below are in the shipped prompts.
- `compare_to_base`: `highlight_lines` on the diff hunks where the change happened, if the reviewer benefits.
- `explain`: `scroll_to` the lines being explained at the start; `highlight_lines` lightly on the range under discussion.

These are *hints*, not enforcement. The LLM still decides. If a verb produces a short, obvious answer, presentation calls are skippable.

## User Controls

### In the Q&A panel

```
─────────────────────────────────────────
 2 highlights · 1 annotation · [Clear all effects]
─────────────────────────────────────────
```

A footer that shows what's currently applied and offers a single Clear button.

### The guided tour

The model fires its highlights while the answer is still streaming, so the
reviewer would otherwise only ever see the end state. Every successful
presentation call is recorded as a *step*, and the panel offers a tour widget
over them: Prev / Next, a step counter, the step's `label` and `detail` beside
the code, and "Show all".

**Scrolling only ever follows a reviewer action.** A live `scroll_to` is
recorded but does not move the viewport; only stepping through the tour or
replaying does. An answer that yanks the page around while the reviewer is
still reading the previous sentence is worse than no navigation at all. The
widget opens *armed* on the first presentation call of a turn — showing
"Scroll to first highlight" and waiting for a click — and opens in normal mode
when the reviewer opens it themselves from the panel.

This replaced a timed replay that paced steps automatically. The pacing was
never right: too fast to read, too slow to skim, and moving the page without
being asked.

### Settings (extension options page)

- `Auto-clear effects on new question` — bool, default on.
- `Allow open_link in new tab` — bool, default on.
- `Allow open_link in embedded panel` — bool, default off (more invasive UX; opt-in).
- `Disable presentation tools` — bool, default off. When on, the agent never sees these tools in its tool list. Useful for users who prefer pure text answers.

### Per-session override

A toggle in the panel header: 🔇 (presentation off for this session). One click and the next question's tool set excludes presentation tools. Stays off until cleared.

## What Presentation Tools Don't Do

- They do not modify code. There is no "apply this fix" tool.
- They do not post to GitHub. They do not create review comments.
- They do not affect anything outside the active PR page (no cross-tab effects, no system-wide notifications).
- They do not run autonomously. Every presentation call is the consequence of a question the user asked.
- They do not survive page navigation. Closing the tab or navigating away clears everything.

## Failure Modes

| Condition | Daemon-side handling | LLM-visible |
|---|---|---|
| Extension WS closed mid-call | Dispatcher returns `ok: false, error: "extension_unavailable"` | LLM sees the error; agent loop continues without retrying |
| File not in current view | Extension returns `file_not_in_view` | LLM can adapt — usually omits or moves on |
| Invalid URL in open_link | Extension returns `url_rejected` | LLM should rephrase without the link |
| Tool used while presentation disabled | Daemon doesn't even register the tool in this turn's tool set | LLM never sees the tool; doesn't try |
| Timeout (5s) | Dispatcher gives up, returns `timeout` | Same as extension_unavailable from agent's POV |

## Storage Implications

Presentation calls are recorded in `tool_traces` just like any other tool — `tool_name`, `input_json`, `output_json`. This means:

- The conversation export can include "the agent highlighted these lines" as part of the investigation context (in verbose export mode).
- Replaying a session (a future feature) could re-apply the same presentation effects.
- Debugging the LLM's behavior is easier — we can see what it tried to call and what came back.

No new schema. Just a new `tool_name` category.

## Threat Model Notes

- `open_link` is the only tool with cross-origin reach. The URL validator must be strict (https or known-safe relative paths; no `javascript:`, `data:`, `file:`, etc.).
- Annotation content comes from the LLM. Treat it as untrusted strings — sanitize before injecting into the DOM. Existing `annotate` code in the POC already uses `textContent`, not `innerHTML`; carry that forward.
- `highlight_lines` color values are an enum on the daemon side — string is mapped to a class name, not interpolated as CSS.
- No tool can execute arbitrary JS in the page context. There is no `eval_in_page` tool and there will not be one.

## Future Tools (Not In Phase B)

Recorded for context, not committed:

- `compare_side_by_side(file_a, line_a, file_b, line_b)` — open a small inline diff between two locations.
- `pin_to_panel(content)` — pin a snippet of the answer to the panel header so it stays visible while the user scrolls the conversation.
- `mark_reviewed(file)` — toggle GitHub's "Viewed" checkbox for a file. Needs the extension to drive GitHub's UI, which is doable but brittle.

Each of these gets added only when there's a real reviewer workflow that warrants it.
