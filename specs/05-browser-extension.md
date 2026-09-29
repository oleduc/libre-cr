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

- **GitHub adapter and selectors** (`src/platform/github/adapter.ts`, `selectors.ts`). The selector versioning and shadow-DOM-piercing approach are correct and worth preserving. Selector list will need refresh by the time we build — GitHub markup changes. (It did: `utils/github/selectors.ts` now covers both the classic server-rendered PR page and the React "changes" UI that `/pull/<n>/files` redirects to — title, base/head refs, per-file `table[aria-label="Diff for: …"]`, `td[data-line-number][data-diff-side]`, and the head SHA from the embedded page JSON. `SELECTOR_VERSION = 2`.)
- **Shadow DOM shell** (`createShadowRootUi`). Style isolation is required.
- **Theme detection** (`src/ui/theme.ts`). GitHub dark/light + system-pref fallback.
- **Floating widget mechanics** — drag, resize, tile, position memory. The Q&A panel is a floating widget. Tiling stays as a power-user feature; the tile grid is genuinely useful when the user wants two PRs open and pinned side-by-side.
- **`EventBus` pattern** — content script ↔ React shell action dispatch with replay buffer.
- **CSP-safe schema validator** — used to validate daemon responses on the extension side.

## What Is Removed Or Replaced

- The function/command runtime (`functions/runtime.ts`, registry, built-in functions). Replaced by direct calls to the daemon's HTTP/WS API.
- Background service worker as LLM call host. Background script is a thin relay for daemon communication (HTTP + WebSocket) — see "Transport from a Content Script" below.
- API key storage. The extension no longer stores or sees the API key; that's the daemon's concern.
- `UIController` as a tool-callable contract. The *contract* is replaced by the presentation-tools layer: the LLM calls a fixed set of presentation tools registered in the review daemon, the daemon routes them back over the WS as `presentation_call` frames, and the extension executes them locally using the POC's existing `UIController` implementations (highlight, annotate, scroll, navigate). The DOM-injection code itself carries over. See `09-presentation-tools.md`.

## Tech Stack

Same as POC: WXT + React + TypeScript. Manifest V3. Anthropic SDK (and friends) are gone; what remains is HTTP + WebSocket + JSON.

Styling is a hand-written CSS string injected into the panel's shadow root, plus a separate page-level sheet for diff effects installed via `adoptedStyleSheets` (CSSOM insertion survives GitHub's `style-src` CSP). There is no Tailwind and no CSS framework: the panel is a few hundred lines of CSS, and the effect styles must key on `data-*` attributes rather than classes anyway (see `09-presentation-tools.md`).

## Manifest Surface

```json
{
  "manifest_version": 3,
  "permissions": ["storage"],
  "host_permissions": ["*://github.com/*", "http://127.0.0.1/*", "http://localhost/*"],
  "content_scripts": [{
    "matches": ["*://github.com/*/pull/*", "*://github.com/*/pull/*/files*"],
    "js": ["content-script.js"],
    "run_at": "document_idle"
  }],
  "background": { "service_worker": "background.js" },
  "options_ui": { "page": "options.html", "open_in_tab": true },
  "action": { "default_popup": "popup.html" }
}
```

`host_permissions` **does** include loopback, and daemon calls **are** relayed through the background service worker. Both were forced by the same discovery: a content script's `fetch` carries the *page's* origin and the page's CSP, and github.com's `connect-src` excludes `127.0.0.1`, so direct calls never left the browser. See § Transport from a Content Script, and `CHANGELOG-TESTING.md` for the diagnosis.

## Transport from a Content Script

The content script **cannot** call `127.0.0.1` directly. Under MV3 a content script's `fetch`/`WebSocket` run with the **page's** origin (`https://github.com`) *and* the page's **CSP** — and github.com's `connect-src` does not include 127.0.0.1, so the request is blocked before it leaves the browser (`securitypolicyviolation`, surfaced as `Failed to fetch`). Found in manual testing; no daemon-side setting can fix a CSP GitHub sets.

All daemon traffic from the content script is therefore relayed through the background service worker, which is bound to neither the page origin nor its CSP and has `host_permissions` for `127.0.0.1`:

- HTTP: `DaemonClient` is constructed with `fetch: daemonFetch()` (`utils/daemon/proxy.ts`), which sends `{url, method, headers, body}` via `runtime.sendMessage`; the worker performs the fetch and returns `{status, statusText, headers, body}`, rebuilt into a `Response`.
- WebSocket: `AskSession` gets `wsFactory: daemonWsFactory()`, a `WSLike` over a `runtime.connect` port; the worker owns the real socket and relays `open` / `message` / `error` / `close` frames. A dropped port surfaces as close code 1006.
- Extension pages (popup, options) keep calling the daemon directly — they run in the extension origin with no page CSP.

The daemon's CORS is `*`; the bearer token is the security boundary. The browser-E2E fixture page sets `connect-src 'self'` so the suite fails if the relay regresses.

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

Reviewers select code in three ways, or pick a review comment. Each produces a structured `Selection` object the extension sends with the question. Every variant may carry the selected text (see `10-grounding-and-context.md` § Evidence on the wire).

```ts
type Selection =
  | { kind: "line",  file: string, line: number, text?: string }
  | { kind: "range", file: string, start_line: number, end_line: number, text?: string }
  | { kind: "symbol", file: string, line: number, column: number, identifier: string, text?: string }
  | { kind: "comment", comment_id: string, file: string, line: number,
      side: "left" | "right", comments: { author: string, body: string }[] };
```

- **Line:** click on a diff line number gutter.
- **Range:** shift-click extends; multi-line drag selection.
- **Symbol:** Cmd/Ctrl-click on an identifier. `pickIdentifier` is a regex over the clicked line's text, with the column derived from the mouse offset across the cell — there is no tree-sitter layer in the extension, minimal or otherwise. The daemon's `find_definition` would resolve it, but that tool is a stub (see `03-code-daemon.md` § Symbols), so a symbol selection is currently only an anchor for the question.

### Review-comment selection

> **Status: built.** Specified before the implementation — unlike most of what
> follows a testing round — then built against it. The two cases the design
> called out as unobserved (a thread with replies, a resolved thread) were
> checked on live PRs before the code landed; see § What the live check
> changed.

A reviewer can pick a **GitHub review comment** — one of the threads that
annotate a source line in the diff — and ask about it: *"is this concern
valid?"*, *"did the reply actually answer it?"* The comment is context for the
question, exactly like a code selection.

**The unit is the thread, not a single comment.** A review thread accumulates
replies, and the reply is frequently where the answer lives ("intentional,
because X"). Capturing only the top comment would routinely drop the half that
resolves the question. `comment_id` is the top comment's id — the thread's
identity for citation and dedup — and `comments` carries the thread oldest
first.

**Why the anchor matters more than the body.** An inline review comment knows
its `file` and `line`, so the selection hands the model both the concern *and*
where it points; the model can then read that location itself with
`get_pr_diff --paths` or `read_file`. That is what makes a separate
multi-item "context basket" unnecessary: the anchor already links comment to
code.

`side` is carried because a comment on a *removed* line has an OLD-side line
number, and resolving it against the new file would read the wrong line. (The
three code variants do not carry side — a pre-existing gap, not widened here.)

**Gesture: a hover affordance, not a click.** Hovering a thread reveals a
small "Ask about this" control; clicking it sets the selection and opens the
panel. A modifier-click was rejected: a comment body is an interactive region
full of links, `Reply` and `Resolve`, and hijacking clicks there is materially
riskier than in a diff cell. The control is our own injected element, so it
competes with nothing.

Injection follows the same discipline as presentation effects
(`09-presentation-tools.md`): marked with `data-libre-cr-*` attributes, since
React rewrites `className`; created on demand from a delegated `mouseover`
rather than pre-injected, because comment threads are **virtualized** — only
the mounted file's threads exist in the DOM at all.

**Both tabs are supported**, and they are different DOMs — the same split as
everywhere else in `selectors.ts`. The Conversation tab is the classic
server-rendered markup: a thread is `.js-resolvable-timeline-thread-container`,
comments are `id="discussion_r<databaseId>"`, the path is the header link's
text, and the thread carries **its own diff hunk** whose *last* numbered row is
the annotated line (`td.blob-num[data-line-number]`; `blob-num-deletion` = old
side). A resolved thread there is collapsed behind `data-deferred-content-url`
with no body in the DOM, so it yields no selection — the same practical limit
as the changes tab, by a different mechanism.

**Verified selectors** for the React changes UI (read from a live PR,
2026-09-10; CSS-module class names such as `ReviewThread-module__…` are
build-hashed and must never be used):

| What | Selector | Verified value |
|---|---|---|
| Thread container | `[data-testid="review-thread"]` | 1 mounted of 20 threads on the PR |
| Comment id | descendant `[id^="r"]` matching `^r\d+$` | `r3872880867` — the same id GitHub's REST API uses |
| Annotated file | enclosing `table[aria-label^="Diff for: "]` | `crates/libre-cr-review/src/provider/anthropic.rs` |
| Annotated line + side | the thread's **own** `tr`, `td[data-line-number][data-diff-side]` | `36` / `right` — matches the API's `line` for that comment |
| Comment body | `.markdown-body` within the thread | present |
| Author | `[data-testid="avatar-link"]` href, or the header's `a[href^="/"]` | `coderabbitai[bot]` |

The line comes from the thread's own row, **not** from the preceding code row.
An earlier draft of this design walked backwards to the previous row and got
`35` for a comment the API places on `36`; the thread row carries the correct
number itself.

Body text is capped at capture like other selection text (4,000 chars), and
the daemon quotes it fenced under the same 2,000-char cap it already applies —
no new knob. The daemon renders the thread as `@author: body` blocks under a
`[Selection: review comment on line N (old|new side) in <file>]` header.

#### What the live check changed

The design named two unobserved cases. Both were checked before the code
landed, on PRs picked for having them (2026-09-10):

- **A thread with replies** — verified on a Kubernetes PR: two comments, two
  `id="r<databaseId>"` roots in document order (oldest first, ascending ids),
  one `.markdown-body` and one `[data-testid="avatar-link"]` each. The thread's
  own `tr` carried `data-line-number="360"` / `data-diff-side="right"`, matching
  the payload's `R360`. Every assumption in the design held.
- **A resolved thread** — *it is not in the DOM at all.* The changes UI renders
  only unresolved threads: a PR with 13 resolved reply threads had zero thread
  elements with the annotated file on screen, and our own PR rendered exactly
  its one unresolved anchored thread. So the hover affordance reaches
  unresolved threads only, and `Selection` carries no `resolved` field —
  nothing selectable is resolved. Resolved threads reach the model the other
  way, through `get_pr_comments`, which reads the payload and does see them.

The affordance is mounted whether or not the Q&A panel is open — the panel
opens on selecting a comment. Requiring the panel first would defeat the
gesture, whose point is to start from the comment.

One case the design missed entirely: a **file-level** review comment
(`markersMap` key `FILE`) renders as a thread with no diff row, so it yields no
`file:line` anchor and therefore no `Selection`. The affordance computes the
selection on hover and stays hidden when there is none, rather than offering a
control whose click does nothing. Selecting a file-level comment would need
`Selection` to allow a missing `line`; that is not built.

Separately from selection, the scraper captures **all** review comments into
`pr_data.comments` for `get_pr_comments` (`04-review-daemon.md` § Internal
Tools). Only the **diff view** carries them: the Conversation tab's embedded
payload has no `pullRequestsChangesRoute` at all, so a scrape there states
nothing about comments — which is not a failure, and is not warned about. The
daemon carries the stored comments forward when a scrape omits them, so opening
the Conversation tab does not delete what the Files tab captured. That path reads the embedded page payload
(`script[type="application/json"][data-target="react-app.embeddedData"]`),
joining `markers.threads` with each `diffSummaries[].markersMap` for the
anchor, because the DOM holds neither the virtualized threads nor the resolved
ones. Selection uses the DOM instead: it needs the element the reviewer is
hovering, which is by definition mounted.

**The selection layer's listeners attach once.** They live on `document`, and
the effect that installs them depends on whether the layer is enabled — never
on the handler's identity. A caller that builds its handler inline (the normal
way to write one) would otherwise re-attach on every render, and re-attaching
re-reads the URL hash and re-emits the selection, which re-renders: a loop that
allocated until Chrome killed the tab. The handler is read through a ref, and
an unchanged selection is not treated as a state change, so the cycle has no
way to start.

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

- **Conversation scrolls.** New turns append at the bottom. Older turns collapse to single-line summaries when a new question lands; click a collapsed summary to expand. Collapse is a *controlled* prop owned by the panel (not seeded once from local component state), so the collapse-older behavior actually re-applies as the conversation grows.
- **Thinking trace** is collapsed by default. Click expands to show the sequence of tool calls + truncated results. The reviewer should *want* to look at this when they're skeptical.
- **Notes** look distinct from Q&A turns: gray background, no thinking trace, simple text.
- **Verbs** are buttons. Clicking one immediately runs the verb against the current selection — no question text required. The result appears as a Q&A turn with the verb's name as the question.
- **Question box** accepts text. Enter submits. Shift-Enter newlines. Each submission is a new WS connection (per `04-review-daemon.md`).
- **Answers render as markdown.** Model output is markdown, and reading it raw
  cost more than it saved. `marked` produces the HTML and an allowlist
  sanitizer walks it before it reaches the DOM — model output must never reach
  `innerHTML` unfiltered, so this is the one place `dangerouslySetInnerHTML` is
  permitted, behind that sanitizer. Links are `http(s)`-only and open in a new
  tab; fenced-code language classes survive, because copy needs them.
- **Copying a rendered answer yields markdown.** The rendered tag set is
  exactly the sanitizer's allowlist, so a small DOM→markdown serializer
  round-trips the selected fragment on `copy`: emphasis, inline and fenced code
  (with language), links, nested lists, tables, blockquotes, headings and
  task boxes go to `text/plain`, with the HTML kept for rich-text targets. A
  partial inline selection stays plain text. Pasting an answer into a review
  comment was otherwise a flattened wall of text.
- **Truncated evidence is announced.** When a `tool_result` frame carries
  `truncated_from`, the trace line is marked (`⚠ truncated from 589,499 chars`)
  and the turn shows a notice naming how many results were shortened and where
  to raise the caps. A shortened answer that looks complete is worse than a
  visible gap — see `10-grounding-and-context.md` § Reporting what was cut.
- **Conversation restores.** The panel rebuilds prior turns from
  `GET /v1/sessions/:id` on load — collapsed Q&As with their selection chips,
  editable notes, and markers for cancelled or failed turns. A session with no
  history starts *closed* behind the floating CR button; only history or an
  error opens it unasked. Restored turns keep their daemon ids so a later
  question can name them in `context_turn_ids`.
- **Keystrokes stay in the panel.** GitHub binds single-key document-level
  hotkeys (`t` focuses its file finder), so the panel stops propagation of key
  events originating inside it. Typing a question must never drive the host
  page.
- **The panel is resizable and re-openable.** Native CSS `resize: both` with the
  size persisted per PR alongside the drag position; closing leaves a floating
  button that brings it back, rather than requiring a page reload.

## Diff Interaction Layer

The diff itself isn't owned by us, but we layer a few things on top:

> **Status:** only the last item is built. The `SelectionLayer` installs a
> single capture `click` listener and renders nothing — there is no hover
> affordance, no popover, and no "show on diff" control. Selection instead
> rides GitHub's own gestures (see § Selection Model). The three unbuilt
> items stay here as intended behaviour.

- **Line highlights.** *Not built.* When a question's answer references specific lines, the panel would offer "show on diff" — clicking scrolls the diff to the file/line and applies a temporary highlight. (The underlying mechanism exists and is used by presentation tools; what is missing is the panel-side affordance.)
- **Reference popovers.** *Not built,* and blocked on `find_references`, which is a stub (`03-code-daemon.md` § Symbols).
- **Selection gutter affordance.** *Not built.* Hovering a diff line number would reveal a small "Ask" button in the gutter.
- **Presentation-tool effects.** During a turn, the LLM may issue `presentation_call` frames over the WS (highlight a line, annotate, scroll, open a link). The extension executes them via the same DOM injection layer. See `09-presentation-tools.md`.

We deliberately do **not** auto-annotate the diff. Annotations come only from user actions (clicking a reference), explicit `add_note` calls (visible in the panel, not the diff), or presentation-tool calls produced by the LLM *in response to* a user question — never preemptively.

## Presentation Handler

A small subsystem in the extension that:

- Subscribes to the active Q&A WebSocket and listens for `presentation_call` frames.
- Validates each call against a hardcoded schema for that tool name.
- Dispatches to the underlying implementation (from the POC's `UIController`).
- Tags every applied effect with `(session_id, turn_id, effect_id)` and tracks it in a session-scoped effect registry.
- Sends back a `presentation_result` frame with `{ ok, result?: { effect_id }, error?, message? }`.

A per-session **🔇 mute** toggle suppresses presentations for the current session. It is not cosmetic: when set, the extension sends `mute_presentations: true` in the WS `AskInit` frame, and the daemon responds by not registering the presentation tools for that turn at all — so the model never emits `presentation_call` frames while muted. As defense in depth the handler also gates locally: any stray `presentation_call` arriving during a muted session is answered with `{ ok: false, error: "presentation_muted" }` rather than executed, so the agent turn still completes. The mute state is persisted per session in `browser.storage.local`.

Effects are cleared on:
- User clicks "Clear all effects" in the Q&A panel footer.
- User submits the next question and the "auto-clear" setting is on (default).
- The user closes the Q&A panel.
- Content script invalidation / navigation away.

The Q&A panel gains a footer showing the count of currently-applied effects and a `Clear all effects` button:

```text
─────────────────────────────────────────
 2 highlights · 1 annotation · [Clear all effects]
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
| `daemon.token` | bearer string | **Plaintext** in `browser.storage.local`; the daemon's authoritative copy is on disk. Not obfuscated or encrypted, and not planned to be: `storage.local` is already origin-isolated to the extension, anything with code execution in that context can read the key material either way, and the token only grants access to a loopback daemon on the user's own machine |
| `daemon.extension_origin` | `chrome-extension://<id>` | What the daemon will allow via CORS |
| `ui.theme_override` | `"system" \| "dark" \| "light"` | Optional |
| `ui.panel_position` | `{ x, y, width, height }` per `pr_url` | Persisted floating widget geometry |
| `session.presentations_muted` | `Record<session_id, bool>` | Per-session 🔇 mute state |
| `ui.protocol_mismatch` | `{ at, daemon, extension }` or absent | Set by the soft protocol-version check; surfaced in Options diagnostics |
| `ui.last_daemon_error` | `{ at, message }` | Most recent daemon failure, for Options diagnostics |
| `ui.last_daemon_ok_at` | epoch ms | Last successful daemon call |
| `ui.diff_change_dismissed` | per `pr_url` | Suppresses the repeat "the diff changed" notice |
| `onboarding.first_pair_seen` | bool | Gates the one-time post-pairing hint |

Nothing about conversations, sessions, or PRs lives here. The daemon is the source of truth.

## Background Service Worker

Thin relay for the content script's daemon traffic (see § Transport from a Content Script — the direct path is blocked by GitHub's CSP):

- `libre-cr/fetch` message → performs the HTTP request, answers `{status, statusText, headers, body}`.
- `libre-cr/ws` port → owns the WebSocket for one ask turn, relays frames both ways; closes the socket when the port disconnects.
- Lifecycle: an open port / active WebSocket keeps the worker alive for the turn; otherwise it can sleep.

## Popup

The extension popup (toolbar icon) shows:

- Daemon status (connected / not paired / unreachable).
- A list of recent sessions across all PRs (top 5).
- A "Configure daemon" link → opens the daemon's config UI in a new tab, with the bearer token appended as `?token=` (`<endpoint>/config-ui?token=<token>`) so the page can authenticate its JSON calls without a separate login.
- A "Pair extension" button → opens the extension's options page.

Useful for jumping back to a PR you reviewed yesterday without having to navigate GitHub.

## Options Page

- **Daemon pairing** (endpoint + token). Accepts a typed pairing code and also handles pairing **deep-links** of the form `#pair?endpoint=<url>&code=<code>[&auto=1]`: the parser reads `location.hash` (not the query string) and requires the `pair` prefix. It pre-fills endpoint and code, and completes pairing without interaction only when `auto=1` is present. This is the default pairing path (option **B** above); manual code entry remains the fallback.
- **Theme override.**
- **Presentation settings** — auto-clear on new question, `open_link` target toggles, and a global "disable presentation tools" switch. *Not built.* The page ships three sections only: Pairing, Theme override, Diagnostics. `allowOpenLinkTab` / `allowOpenLinkPanel` are hardcoded defaults with no UI, and `autoClearOnNewQuestion` is declared but never read — clearing on a new question is unconditional. The per-session 🔇 mute in the panel header *is* shipped and covers the "disable presentations" need for now.
- **Per-PR panel reset** (clears stored positions). *Not built.*
- **Diagnostics** (last daemon error, time of last successful call, and any protocol-version mismatch recorded by the soft health check).

Provider/LLM/API-key config is **not** here. That's on the daemon's config UI.

### Protocol-version check

On session init the extension reads `protocol_version` from `GET /v1/health` and compares it to its own `PROTOCOL_VERSION` constant (mirrored from `libre-cr-common`). A mismatch never blocks anything — minor versions are wire-compatible by spec — it logs a console warning and records `ui.protocol_mismatch` for the Options diagnostics panel. A missing field (an older, pre-versioning daemon) is treated as compatible.

## Error Surfaces

> **Status: aspirational.** The panel's state union is
> `loading | not_paired | preparing | ready | error` and every branch renders
> plain text. There is no toolbar pill, no retry button, no elapsed estimate
> and no "report mismatch" link. The table is the target; what ships today is
> the message, not the affordance. Two rows *are* real in substance: a
> not-paired session shows a pairing prompt, and a turn error renders inline
> in the turn.

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
