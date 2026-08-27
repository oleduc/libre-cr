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

The mechanism (DOM injection, namespaced effects, the existing `UIController` implementations from the POC) carries over. The product semantics are different.

## Tool Catalog (Phase B)

Five tools. Conservative on purpose — we add more after watching real use.

### `highlight_lines`

```
highlight_lines(file, start_line, end_line, color?: enum, label?: string)
```

Visually highlight a line or range in the diff. Same overlay technique as the POC. `color` defaults to a neutral blue; the agent can pick from `red`, `yellow`, `green`, `blue`, `purple` for cue value. `label` is a tooltip.

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

Open a URL. `target="tab"` opens in a new browser tab (default). `target="panel"` opens it in a small embedded iframe panel within the Q&A widget when the URL is from a known-safe origin (GitHub-hosted issues, blob URLs, etc.).

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
  "result": { "applied": true, "effect_id": "h_xyz" } }
```

Or on failure:

```json
{ "type": "presentation_result",
  "call_id": "p_abc123",
  "ok": false,
  "error": "file_not_in_view",
  "message": "src/auth.ts is not currently in the diff view." }
```

The daemon treats the `presentation_result` exactly like a regular tool result — feeds it back to the LLM as the tool result block. If the extension reports a failure (e.g., file not visible), the LLM sees that and can adapt (e.g., explain in text instead).

### Result envelope

Successful results always include an `effect_id` so the extension and daemon can later reference the effect (e.g., for `clear_presentation`). Effects are also namespaced by `turn_id` so we can scope clearing.

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
  • Closes the panel        → effects cleared
  • Navigates away          → effects cleared automatically on content script unload
```

Reasonable defaults: auto-clear when the next question is asked, manual override available. Effects from notes (manually added by the user) are not cleared automatically — those are reviewer-curated.

## Extension Implementation

Two roles in the extension:

1. **Presentation handler.** Listens for `presentation_call` frames on the active WS. Looks up the tool name in a registry, validates the input shape, executes via the underlying UIController-style functions, replies with `presentation_result`.

2. **Effect bookkeeping.** Tracks every effect by `(turn_id, effect_id)`. Renders the Q&A panel's "X effects applied · [Clear]" footer. Implements the auto-clear logic on new-question / panel-close / nav.

The underlying highlight/annotate/scroll implementations are the POC's `UIController` code, minus the tool-callable wrapper. The POC code stays in place; what changes is who calls it (the extension itself, via the presentation handler, in response to daemon frames).

## Daemon Implementation

Tool router gains a third dispatcher:

```rust
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
- `related_tests`: scroll to the first cited test file at the end (so the reviewer lands somewhere useful), `annotate_line` only if there's a specific concern.
- `compare_to_base`: `highlight_lines` on the diff hunks where the change happened, if the reviewer benefits.
- `explain`: `scroll_to` the lines being explained at the start; `highlight_lines` lightly on the range under discussion.

These are *hints*, not enforcement. The LLM still decides. If a verb produces a short, obvious answer, presentation calls are skippable.

## User Controls

### In the Q&A panel

```
─────────────────────────────────────────
 2 highlights · 1 annotation · [Clear highlights]
─────────────────────────────────────────
```

A footer that shows what's currently applied and offers a single Clear button.

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
