# Browser Extension

## Role

The thinnest of the three components. Its job is:

1. Detect PR pages and scrape their content from the DOM.
2. Pair with the local review daemon and authenticate.
3. Provide selection-driven UI in the diff view.
4. Open a Q&A panel that streams answers from the daemon.
5. Show conversation history and notes for the current PR.
6. Hand off review drafts to the GitHub review composer (clipboard or DOM injection).

The extension owns no LLM logic, no code intelligence, no conversation persistence. If the daemon is not reachable, the extension shows a banner and degrades gracefully — the PR view itself is unaffected.

## What Carries Over From The POC

These elements ported as-is or with minor edits:

- **GitHub adapter and selectors** (`src/platform/github/adapter.ts`, `selectors.ts`). The selector versioning and shadow-DOM-piercing approach are correct and worth preserving. Selector list will need refresh by the time we build — GitHub markup changes.
- **Shadow DOM shell** (`createShadowRootUi`). Style isolation is required.
- **Theme detection** (`src/ui/theme.ts`). GitHub dark/light + system-pref fallback.
- **Floating widget mechanics** — drag, resize, tile, position memory. The Q&A panel is a floating widget. Tiling stays as a power-user feature; the tile grid is genuinely useful when the user wants two PRs open and pinned side-by-side.
- **`EventBus` pattern** — content script ↔ React shell action dispatch with replay buffer.
- **CSP-safe schema validator** — used to validate daemon responses on the extension side.

## What Is Removed Or Replaced

- The function/command runtime (`functions/runtime.ts`, registry, built-in functions). Replaced by direct calls to the daemon's HTTP/WS API.
- Background service worker as LLM call host. Background script becomes a thin proxy for daemon communication (or is removed if content scripts can hold WS connections directly — see "Transport from a Content Script" below).
- API key storage. The extension no longer stores or sees the API key; that's the daemon's concern.
- `UIController` as a tool-callable contract. The *contract* is replaced by the presentation-tools layer: the LLM calls a fixed set of presentation tools registered in the review daemon, the daemon routes them back over the WS as `presentation_call` frames, and the extension executes them locally using the POC's existing `UIController` implementations (highlight, annotate, scroll, navigate). The DOM-injection code itself carries over. See `09-presentation-tools.md`.

## Tech Stack

Same as POC: WXT + React + TypeScript. Manifest V3. Tailwind in Shadow DOM via WXT's CSS injection mode. Anthropic SDK (and friends) are gone; what remains is HTTP + WebSocket + JSON.

## Manifest Surface

```json
{
  "manifest_version": 3,
  "permissions": ["storage"],
  "host_permissions": ["*://github.com/*"],
  "content_scripts": [{
    "matches": ["*://github.com/*/pull/*", "*://github.com/*/pull/*/files*"],
    "js": ["content-script.js"],
    "run_at": "document_idle"
  }],
  "background": { "service_worker": "background.js" },
  "options_page": "options.html",
  "action": { "default_popup": "popup.html" }
}
```

`host_permissions` does not include `127.0.0.1` — we'll use `fetch` from the content script's origin, which has `connect-src` semantics. The daemon's CORS handling allows the extension origin explicitly. If we hit issues we can add `connect-src` via `host_permissions` or move daemon calls to the background script (where `host_permissions` apply).

## Transport from a Content Script

Calling `127.0.0.1` from a content script's page-origin context is subject to CORS but otherwise works in modern browsers. The daemon explicitly allows the extension origin via the pairing dance. WebSocket has the same origin model.

If a future browser tightens this, the fallback is to proxy all daemon traffic through the service worker, which has `host_permissions` for `127.0.0.1` and is not bound to the page origin. We design the daemon-client TS module so it can be hosted in either context.

## First-Run Pairing

```
1. User installs extension. Opens any PR page.
2. Content script tries to read endpoint + token from browser.storage.local.
3. Not found → mount a "Not paired" banner in the floating widget.
4. User clicks "Pair with daemon" → options page opens.
5. Options page:
     • Shows command to start daemon if not running:
         brew install libre-cr   # see distribution spec
         libre-cr start
     • Shows pairing flow: "Run `libre-cr pair` and paste the code below"
     • One-time pairing code field.
6. User submits code. Extension POSTs to http://127.0.0.1:<discovered port>/v1/pair
   with the code. (Port discovered by trying a small set of conventional ports +
   reading ~/.config/libre-cr/endpoint via a "open the file" download dance, or
   via a manual paste of "endpoint URL" if filesystem access isn't workable.)
7. Daemon responds with the bearer token and confirms the extension origin.
8. Extension persists endpoint + token in browser.storage.local.
9. Banner clears; Q&A panel becomes available.
```

The "extension reads the endpoint file" step is the awkward part. Two options:

- **A: Manual paste.** Daemon prints the endpoint URL to its console / config UI. User pastes it into the extension's pairing screen alongside the pairing code.
- **B: Bridge via a small launcher page.** Daemon's config UI (running on its own port) generates a deep link like `<extension-id>://pair?endpoint=...&code=...` that the user opens. The extension's options page handles the deep link and completes pairing automatically.

We default to **B**, fall back to **A** when **B** isn't workable. Manual paste is always available as the "I know what I'm doing" path.

## Content Script Lifecycle

```
content-script load
  ─→ Wait for hydration (existing waitForElement pattern)
  ─→ Detect platform; bail if not a PR page
  ─→ Read endpoint + token from storage; if missing, show "Not paired" banner
  ─→ Scrape PR data: context (owner/repo/number/branches), description, comments, diff
  ─→ POST /v1/sessions with the scraped data
       • Receive session_id, worktree_ready, repo_local_path
       • If !worktree_ready: poll /v1/sessions/:id until it is, with a sensible cap
  ─→ Mount Shadow DOM UI:
       • Floating "CR" button anchored to the PR header
       • Diff selection observers (track user selection in diff tables)
  ─→ Wire SPA navigation: re-detect, re-init, re-mount on PR change
  ─→ Wire cleanup on unload / invalidation
```

The "CR" button stays minimal — it indicates connectivity and opens the Q&A panel. We do not auto-open the panel; reviewers are reading code first, asking questions second.

## Selection Model

Reviewers select code in three ways. Each produces a structured `Selection` object the extension sends with the question.

```ts
type Selection =
  | { kind: "line",  file: string, line: number }
  | { kind: "range", file: string, start_line: number, end_line: number }
  | { kind: "symbol", file: string, line: number, column: number, identifier: string };
```

- **Line:** click on a diff line number gutter.
- **Range:** shift-click extends; multi-line drag selection.
- **Symbol:** hover-over-or-Cmd-click on an identifier. The extension's tree-sitter-lite layer (a minimal TS port for picking the identifier under the cursor) identifies the token; the daemon's `find_definition` resolves it.

The selection is sticky — it persists until cleared or replaced. The Q&A panel header shows the current selection ("`src/auth.ts:42-48` selected · [×]"). Asking a question without a selection is allowed (it's just "ask about this PR").

## Q&A Panel

A floating widget tied to the current session. Three regions:

```
┌──────────────────────────────────────────────────┐
│ [≡] PR #123 — feat: bcrypt migration        × □  │   ← title bar, drag handle
├──────────────────────────────────────────────────┤
│ Selection: src/auth.ts:42–48               [×]   │   ← current selection chip
├──────────────────────────────────────────────────┤
│                                                   │
│ Conversation                                      │
│ ┌──────────────────────────────────────────┐    │
│ │ Q:  why is md5 still in this file?       │    │
│ │ A:  …streaming text…                      │    │
│ │     [▾] thinking trace (3 tool calls)    │    │
│ └──────────────────────────────────────────┘    │
│ ┌──────────────────────────────────────────┐    │
│ │ Note: legacy hash path — see PR #99      │    │
│ └──────────────────────────────────────────┘    │
│                                                   │
├──────────────────────────────────────────────────┤
│ Verbs                                             │
│ [Find callers] [Show history] [Related tests]    │
│ [Compare to base] [Explain]                       │
├──────────────────────────────────────────────────┤
│ ┌────────────────────────────────────────────┐  │
│ │ Ask a question about the selection...      │  │
│ └────────────────────────────────────────────┘  │
│                              [Add note] [Ask ▶]  │
└──────────────────────────────────────────────────┘
```

Behavior:

- **Conversation scrolls.** New turns append at the bottom. Old turns collapse to single-line summaries; click to expand.
- **Thinking trace** is collapsed by default. Click expands to show the sequence of tool calls + truncated results. The reviewer should *want* to look at this when they're skeptical.
- **Notes** look distinct from Q&A turns: gray background, no thinking trace, simple text.
- **Verbs** are buttons. Clicking one immediately runs the verb against the current selection — no question text required. The result appears as a Q&A turn with the verb's name as the question.
- **Question box** accepts text. Enter submits. Shift-Enter newlines. Each submission is a new WS connection (per `04-review-daemon.md`).

## Diff Interaction Layer

The diff itself isn't owned by us, but we layer a few things on top:

- **Line highlights.** When a question's answer references specific lines, the panel can offer "show on diff" — clicking scrolls the diff to the file/line and applies a temporary highlight. Same DOM injection technique the POC used; namespaced by `data-libre-cr-*` attributes for cleanup.
- **Reference popovers.** When the agent uses `find_references` and surfaces line numbers in its answer, we render the line numbers as clickable links that scroll-to + highlight.
- **Selection gutter affordance.** Hovering a diff line number reveals a small "Ask" button in the gutter that opens the panel with the line preselected.
- **Presentation-tool effects.** During a turn, the LLM may issue `presentation_call` frames over the WS (highlight a line, annotate, scroll, open a link). The extension executes them via the same DOM injection layer. See `09-presentation-tools.md`.

We deliberately do **not** auto-annotate the diff. Annotations come only from user actions (clicking a reference), explicit `add_note` calls (visible in the panel, not the diff), or presentation-tool calls produced by the LLM *in response to* a user question — never preemptively.

## Presentation Handler

A small subsystem in the extension that:

- Subscribes to the active Q&A WebSocket and listens for `presentation_call` frames.
- Validates each call against a hardcoded schema for that tool name.
- Dispatches to the underlying implementation (from the POC's `UIController`).
- Tags every applied effect with `(session_id, turn_id, effect_id)` and tracks it in a session-scoped effect registry.
- Sends back a `presentation_result` frame with `{ ok, result?: { effect_id }, error?, message? }`.

Effects are cleared on:
- User clicks "Clear all" in the Q&A panel footer.
- User submits the next question and the "auto-clear" setting is on (default).
- The user closes the Q&A panel.
- Content script invalidation / navigation away.

The Q&A panel gains a footer showing the count of currently-applied effects and a `Clear all` button:

```
─────────────────────────────────────────
 2 highlights · 1 annotation · [Clear all]
─────────────────────────────────────────
```

Settings (options page) for presentation behavior:
- Auto-clear on new question (default: on).
- Allow `open_link` to new tab (default: on).
- Allow `open_link` to embedded panel (default: off).
- Disable presentation tools globally (default: off — when on, the daemon excludes them from the agent's tool set).

## State In `browser.storage.local`

| Key | Value | Notes |
|---|---|---|
| `daemon.endpoint` | `http://127.0.0.1:<port>` | Resolved during pairing |
| `daemon.token` | bearer string | Stored encrypted with the extension's own obfuscation; the daemon's authoritative copy is on disk |
| `daemon.extension_origin` | `chrome-extension://<id>` | What the daemon will allow via CORS |
| `ui.theme_override` | `"system" \| "dark" \| "light"` | Optional |
| `ui.panel_position` | `{ x, y, width, height }` per `pr_url` | Persisted floating widget geometry |

Nothing about conversations, sessions, or PRs lives here. The daemon is the source of truth.

## Background Service Worker

Minimal in v2:

- `OPEN_OPTIONS_PAGE` → opens options.
- (Optional) Proxy for daemon traffic if a future browser forbids the content script direct path.
- Lifecycle: not much. The worker can sleep.

## Popup

The extension popup (toolbar icon) shows:

- Daemon status (connected / not paired / unreachable).
- A list of recent sessions across all PRs (top 5).
- A "Configure daemon" button → opens daemon's config UI in a new tab.
- A "Pair extension" button → opens the extension's options page.

Useful for jumping back to a PR you reviewed yesterday without having to navigate GitHub.

## Options Page

- **Daemon pairing** (endpoint + token).
- **Theme override.**
- **Per-PR panel reset** (clears stored positions).
- **Diagnostics** (last daemon error, time of last successful call).

Provider/LLM/API-key config is **not** here. That's on the daemon's config UI.

## Error Surfaces

| Condition | UI |
|---|---|
| Daemon not paired | Banner with "Pair daemon" CTA |
| Daemon unreachable | Toolbar pill turns gray, banner: "Daemon offline · [Retry]" |
| Worktree pending | Q&A panel header: "Preparing repo… (~Ns)" |
| Worktree failed (private PR) | Q&A panel: error block with specific message + retry button |
| Provider error during turn | Inline in the Q&A turn: "Error: <message> · [Retry]" |
| Selector breakage (scrape returns nothing) | Soft warning + opens a small "report mismatch" link |

## Testing Surface

- **Fixture tests** for the GitHub adapter (carry over the existing approach).
- **Daemon API mock** for the content script. The Q&A flow can be exercised end-to-end against a mock that returns canned stream frames.
- **Pairing flow tests** simulate the options-page handshake.

We do not unit-test floating-widget mechanics beyond what's already covered in the POC. The Q&A panel itself is small enough that integration tests cover it adequately.

## Performance Targets

- Scrape PR data and POST `/v1/sessions`: <300 ms after DOM ready, for a typical PR.
- Open Q&A panel from button click: <50 ms.
- First token of answer visible in panel: bounded by LLM TTFB (typically ~500–1500 ms for current frontier models) + ~10–20 ms transport. The daemon's tool dispatch may add a few hundred ms before the first text token if the model leads with a tool call, but in that case the panel shows the tool call frame immediately.

## Privacy Posture

The extension sends to the daemon: scraped PR data, user's questions, user's notes. It never sends to anywhere else. Selection content is included as part of the question payload — if the user has, e.g., a token in the diff (which shouldn't happen but we don't validate), it would be sent to the configured LLM provider by the daemon. This is the same risk as using any LLM-backed code tool and we don't pretend otherwise.

## What This Extension Doesn't Do

- It does not edit code.
- It does not annotate the diff autonomously.
- It does not store any PR/conversation data locally.
- It does not call any service other than the configured local daemon.
- It does not authenticate to GitHub or use the GitHub API.
