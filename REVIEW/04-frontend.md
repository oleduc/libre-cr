# Frontend / extension review — libre-cr v2

## Summary

The extension is small (~1700 LOC + 924 LOC tests, 46 test cases) and disciplined. Strict TS with `noUncheckedIndexedAccess`, `tsc --noEmit` is clean, and there are zero raw `any` casts in product code. The Shadow-DOM mount, presentation handler safety (`textContent`, color enum, scheme allowlist), structured `Selection` typing, and selector isolation are well-executed. Most spec items in `05-browser-extension.md` and `09-presentation-tools.md` are implemented as written.

Two real issues block a clean v2: (1) the presentation manager is leaked across PR navigations and the WS listener detacher is never invoked, and (2) the popup renders search snippets with `dangerouslySetInnerHTML` of unsanitized daemon FTS output. Several other items are spec drift or accessibility gaps worth fixing.

## Critical (block v2)

### C1 — Popup snippet rendering injects daemon-controlled HTML
`extension/entrypoints/popup/Popup.tsx:131-134` uses `dangerouslySetInnerHTML={{ __html: escapeHtml(r.snippet) }}`. The comment correctly notes that SQLite `snippet()` returns `[...]` markers, but `escapeHtml` only escapes `&<>` — it does not escape quotes, and more importantly there's no need for `dangerouslySetInnerHTML` here at all because the snippet is rendered as plain text. The spec calls for `[match]` bracket markup to be styled (specs/07 § Search) and that's what the comment implies the code is doing, but the current implementation simply escapes everything and then re-parses it as HTML — gaining nothing and creating a latent XSS surface the moment a future contributor changes the daemon-side snippet escaping. Render as plain text or convert `[...]` to a `<mark>` via a typed parser.

### C2 — Presentation manager + WS listeners never torn down
`extension/components/QaPanel.tsx:40` creates the `PresentationManager` once via `useRef` but there is no effect-return that calls `presentationRef.current.detachAll()` on unmount. `attach()` (`utils/presentation/index.ts:87-93`) pushes a detacher into `state.detachers` on every `runAsk` call; over a long session this grows monotonically. More importantly, when the content script tears down on `turbo:render` (entrypoints/content/index.ts:18-27), the manager is GC-eligible but any unresolved `AskSession` may still be holding its socket open and the in-DOM presentation effects (highlights) live on the GitHub page, not inside the Shadow root — they are not cleared because nothing invokes `clearAll()` on unmount. Spec `09 § Effect Lifecycle` explicitly requires "Content script invalidation / navigation away → effects cleared automatically on content script unload". Today, only `pagehide` runs the teardown, and on a Turbo SPA nav between PRs there is no `pagehide`; the highlights left on file rows in the previous PR's diff DOM are orphaned.

### C3 — `AskSession.close()` doesn't reset `inflight` consistently
`utils/daemon/ws.ts:175-186` sets `this.inflight = false` only inside `close()`, but the promise returned by `open()` resolves/rejects from inside `onmessage`/`onclose` without flipping `inflight`. The QaPanel calls `session.close()` in the `finally`, so it currently works, but if anything throws between the promise settling and the `finally`, the session is permanently unusable. Defensive fix: clear `inflight` in the settle path as well. (Borderline — listing here because it makes the lifecycle harder to reason about.)

## Important

### I1 — Shell `style` ignores persisted height
`components/Shell.tsx:62-69` reads `width` from `pos` but never applies `height`, even though `setPanelPosition` is called with the rect dimensions. Stored geometry is half-applied across reloads.

### I2 — Drag persists position regardless of mouse up location
`Shell.tsx:54` always saves `pos` on `mouseup`. There is no clamp to viewport, so dragging the panel off-screen permanently strands the panel for that PR. The spec doesn't require clamping but it's a foot-gun. Also `onTitleDown` adds `mousemove`/`mouseup` listeners to `window` — if a Turbo nav tears down the React tree mid-drag, those listeners stay attached. Cleanup the `dragState` ref via an effect's cleanup or use a pointer-capture pattern.

### I3 — `SelectionLayer` swallows clicks during capture
`components/SelectionLayer.tsx:55` registers `click` with `capture: true` on `document`. Plain clicks on the diff line number gutter set a `line` selection, which is fine — but the listener does not `stopPropagation`. The user's click also reaches GitHub's own gutter behavior (e.g., the "+" comment affordance, anchor links to `#R42`). Worse, the listener fires on every click in the document, not just within the diff; the `hitTestLine` early-out is fine for perf but the `metaKey`/`ctrlKey` branch (Cmd-click) will *also* hijack Cmd-clicks the user makes anywhere inside a diff row to identify a symbol — this conflicts with the OS-level "open in new tab" gesture if the user Cmd-clicks a link inside a diff cell. At minimum, only handle the click when the target is inside `td.blob-num` or scope to `[data-tagsearch-path]`.

### I4 — Spec drift: pairing protocol
`05-browser-extension.md § First-Run Pairing` describes Option B (deep-link/launcher) as the default with manual paste as fallback. The Options page implements only manual paste (`entrypoints/options/Options.tsx:81-100`). That's a perfectly defensible v2 scope, but worth flagging — the spec calls for the deep-link path.

### I5 — Spec drift: per-session "presentation off" toggle missing
`09-presentation-tools.md § Per-session override` requires a 🔇 toggle in the panel header to disable presentation tools for the current session. Not implemented in `QaPanel.tsx`. Also missing: the "Allow open_link in embedded panel" options setting (`Options.tsx` only handles pairing/theme/diagnostics).

### I6 — Spec drift: thinking trace not collapsed by default per turn
Per spec, the **active** turn is expanded; **previous** turns collapse to single-line summaries. `QaPanel.tsx:118-120` does collapse prior QA turns, which is good. But the thinking trace itself (`ConversationTurn.tsx:194`) defaults to `open={false}` only because `expanded` starts `false` — this matches spec. Fine on review.

### I7 — Focus management on panel open
The panel auto-mounts and the textarea is not auto-focused. The spec doesn't require it, but A11y/demo path: when the user clicks the verb buttons or opens the panel via keyboard, focus stays on the trigger. No focus trap on the export modal either (`ExportModal.tsx:89`) — `role="dialog"` is set but Esc-to-close, focus restore, and tab containment are absent.

### I8 — A11y: live region for streaming answers
The conversation pane has no `aria-live` annotation. Screen readers won't announce the streaming answer text. Adding `aria-live="polite"` on `.libre-cr-conversation` (or on each in-flight `.a` div) would meaningfully improve the demo.

### I9 — `?token=` WS auth fallback notes
`utils/daemon/ws.ts:147` builds `?token=...` for the WS upgrade. Token leaks: (1) the browser URL bar never shows it (WS URLs don't appear in history); (2) it *does* show in DevTools "Network → WS" pane and in any extension that intercepts requests; (3) it shows in the daemon's access log unless the daemon strips query strings on log. The spec ack'd this trade-off implicitly. Worth a sentence in code: the daemon should log path-only, not full URL. Not a code-side bug — flagging for the daemon team and for a defensive comment.

## Suggestions

### S1 — Bundle composition
Content script is 196 kB (React DOM + everything). The popup/options chunks share a 147 kB React vendor chunk via WXT's split. React alone is ~140 kB; the rest of `content.js` is ~55 kB of actual extension code, which is reasonable. The cheapest win is replacing React DOM in the content script with Preact (~10 kB) — `react-dom/client` API is fully covered by `preact/compat`. Not required; flagging because the spec mentions "~196 kB per the build report" as something to scrutinize.

### S2 — `styles.ts` as a single string is fine, but unscoped
The CSS does not use `:host`-scoped selectors meaningfully (line 5 `:host, :root { all: initial }` is a no-op for `:root` inside a shadow). All `.libre-cr-*` classes assume they're inside the shadow root, which is true today. Adding a `:host { contain: layout style; }` would harden against any future leak.

### S3 — `parseServerFrame` only checks `type`
`utils/daemon/frames.ts:54-65` validates the discriminator and otherwise blesses the object. A malicious or bug-introduced daemon could send `{type: "text_delta"}` with `text` being a number, and the panel would render `String(undefined)` or worse if `text` were an object. Cheap improvement: per-type field validators. The spec mentions a "CSP-safe schema validator was carried over from POC" — that's a gap.

### S4 — Memoization
None used. The component tree is small enough that it doesn't matter today, but `runAsk` re-creates on `props.selection` change, which is fine. No false performance fixes needed.

### S5 — `dispatchPresentationCall` type casts
`utils/presentation/handlers.ts:236-252` casts `input` via `Parameters<typeof ...>[1]`. Functionally fine because each handler revalidates, but a discriminated input shape per tool (which the Rust side already encodes) would let TypeScript do that work for free.

### S6 — `pollUntilReady` cannot be cancelled
`components/ContentApp.tsx:74-81` and `pollUntilReady:146-167` — the `cancelled` flag suppresses the `done` callback, but the inner `await new Promise((res)=>setTimeout(res, delay))` continues to schedule timeouts up to 60s after unmount. Not a leak that matters, but a `clearTimeout` would be tidy.

### S7 — `addNote` happy-path doesn't show the note while in-flight
`QaPanel.tsx:199-212` waits for the daemon's `note_id` before appending to `turns`. Optimistic insertion would feel snappier; rollback on failure.

## Confirmed good

- **TypeScript strictness.** Strict mode + `noUncheckedIndexedAccess`. Zero raw `any` in product code; the few `unknown` casts are at trust boundaries (storage, globalThis, frame parse). `tsc --noEmit` passes clean.
- **`Selection` discriminated union** (`utils/selection.ts`) matches Rust's `libre-cr-common::Selection` shape; `selectionSatisfies` is total and switchable.
- **Presentation handler safety.** `textContent` for both summary and detail (`handlers.ts:120,125`), color is constrained to `VALID_COLORS` and mapped to a class name (`colorClass:46`), no CSS string interpolation. The `isSafeUrl` test cases cover `javascript:`, `data:`, `file:`, and protocol-relative `//evil.com`.
- **Selector isolation.** All GitHub selectors live in `utils/github/selectors.ts` with multi-fallback selectors and a `SELECTOR_VERSION`. Scrape is defensive — every miss becomes a warning, never an exception (`scrape.ts:16-42`).
- **Effect tagging.** Every applied effect has `data-libre-cr-effect-id` + `data-libre-cr-tag`; `clearPresentation` removes by tag (`handlers.ts:188-218`).
- **Storage hygiene.** Typed `StorageShape`, in-memory fallback for tests, no PII (per spec — sessions/conversations live in the daemon).
- **Content script lifecycle.** Turbo `turbo:render`/`turbo:load` re-init, `pagehide` teardown. Aborted async via `cancelled` flag. React 18 `Root.unmount` called in `tearDown`.
- **Test coverage.** 46 specs across the WS protocol, HTTP client, GitHub adapter (scrape + hit-test + pickIdentifier), presentation handlers (including the `<script>` escape and scheme rejection cases), QaPanel render + verb gating, export modal, note CRUD, popup search.
- **`AskSession` typed handlers.** `on<T>` returns the exact frame variant via `Extract<ServerFrame, {type: T}>` — clean.

## Coverage notes

- **End-to-end read:** `entrypoints/content/index.ts`, `components/ContentApp.tsx`, `components/Shell.tsx`, `components/QaPanel.tsx`, `components/ConversationTurn.tsx`, `components/SelectionLayer.tsx`, `components/ExportModal.tsx`, `components/styles.ts`, `utils/daemon/{client,ws,frames,storage,pairing}.ts`, `utils/presentation/{handlers,index}.ts`, `utils/github/{detect,scrape,diff,selectors}.ts`, `utils/selection.ts`, `entrypoints/popup/Popup.tsx`, `entrypoints/options/Options.tsx`, `entrypoints/background.ts`, `wxt.config.ts`, `tsconfig.json`, `package.json`, `tests/presentation-handlers.test.ts`.
- **Spot-checked:** `tests/qa-panel.test.tsx` (first 60 lines), test file inventory + describe blocks for the other 13 test files, built `.output/chrome-mv3` for size/composition.
- **Did not read in depth:** every other test file's assertions (read the describe blocks only); the popup-search-results UX. Compared `extension/.output/chrome-mv3/manifest.json` against the spec's manifest snippet — matches with `host_permissions` expanded to include `127.0.0.1`/`localhost`.
- **Tooling:** ran `pnpm typecheck` (clean), did not run `pnpm test` or `pnpm build` — build artifacts already present.
