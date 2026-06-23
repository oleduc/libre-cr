# Extension review — round 2

## Summary (verdict)

The three round-1 criticals are genuinely fixed, with real tests behind them, and the new browser E2E suite is unusually good — it spawns a real daemon, drives a real Chromium with the built MV3 bundle, and asserts page-DOM effects, not just element presence. `tsc --noEmit` is clean and all 60 unit tests pass (2.3 s). However, three round-1 *important* items (I13, I14, I17) are untouched, the I16 mute toggle is **functionally a no-op** (the daemon ignores the flag and the extension doesn't gate locally — admitted in a code comment), and I found one new behavioral bug: auto-collapse of previous Q&A turns is broken because `ConversationTurn` seeds collapse state from props once and never re-syncs. **Verdict: conditional pass — no criticals; ship-blocking only if the mute toggle's no-op status is unacceptable for certification (a visible control that does nothing).**

## Critical

None.

## Important

1. **Mute toggle is a placebo (I16 partial).** The 🔇 button persists per-session state and sends `mute_presentations` in `AskInit` (`extension/components/QaPanel.tsx:206`), but `extension/utils/daemon/frames.ts:12-19` admits "The daemon does not yet honor this flag (TODO on the Rust side)" — `grep -r mute_presentations crates/` confirms zero hits. The extension also doesn't gate locally: the `PresentationManager` is attached regardless of mute (`QaPanel.tsx:170`) and `handle()` executes every `presentation_call` (`utils/presentation/index.ts:54-77`). Muting changes nothing observable. Cheap fix: skip `dispatchPresentationCall` (reply `ok:false`) when muted. The test (`tests/qa-panel-a11y-mute.test.tsx:47-70`) only asserts persistence, never suppression — exactly the "makes a test pass" pattern this round was meant to catch.
2. **Auto-collapse of prior turns is broken (new).** `runAsk` marks earlier turns `collapsed: true` (`QaPanel.tsx:151-154`), but `ConversationTurn.tsx:71` does `useState(turn.kind === "qa" && turn.collapsed === true)` — `useState` initializers run once, keys (`t.id`) are stable, and there is no syncing effect, so already-mounted turns never collapse when a new question is asked. Spec 07's "previous turns collapse to single-line summaries" is silently dead; no test covers it (`grep collaps tests/` → nothing).
3. **I13 — Shell geometry: NOT fixed.** `components/Shell.tsx:62-69` still applies only `top/left/width`; persisted `height` is written but never read back. No viewport clamp (`Shell.tsx:40-47` — drag off-screen permanently strands the panel for that PR). `mousemove`/`mouseup` window listeners (`Shell.tsx:58-59`) still leak if the tree unmounts mid-drag (no effect cleanup / pointer capture). Bonus: `setPanelPosition` is called inside a `setPos` updater (`Shell.tsx:53-56`) — a side effect that double-fires under StrictMode.
4. **I14 — document-wide click hijack: NOT fixed.** `components/SelectionLayer.tsx:55` still capture-listens on every document click; `hitTestLine` (`utils/github/diff.ts:65-82`) matches any node inside any `<tr>` under `[data-tagsearch-path]`, so plain/shift/cmd clicks anywhere in a diff row (code cells, links, GitHub's own affordances) mutate the selection. Still not scoped to `td.blob-num`.
5. **I17 — open_link settings: NOT fixed.** `allowOpenLinkTab`/`allowOpenLinkPanel` exist only as context defaults (`utils/presentation/handlers.ts:22-33`); `QaPanel.tsx:41` creates the manager with no options, there's no storage key, and `Options.tsx` still exposes only pairing/theme/diagnostics.

## Suggestions

- **Deep-link endpoint unvalidated.** `parsePairDeepLink` (`entrypoints/options/Options.tsx:21-34`) accepts any `endpoint=` and auto-pairs without confirmation (`Options.tsx:78-81`). Risk is low (no `web_accessible_resources` in the manifest, so web pages can't navigate to `options.html`; the user must paste the URL), but restricting auto-pair to loopback endpoints is one line. Note the flip side: because the page isn't web-accessible, the daemon config UI's generated link can't be *clicked* — copy-paste only, which undercuts spec path B's UX.
- **`renderSnippet` brackets** (`entrypoints/popup/Popup.tsx:180-203`): regex `\[([^\]]+)\]` handles unbalanced (`[abc` → plain text) and empty (`[]` → literal) safely; nested `[[foo]]` mis-marks `[foo` — cosmetic only, inherent to FTS bracket ambiguity, never a parse surface. Fine as-is.
- **Detacher growth**: `attach()` per `runAsk` pushes into `state.detachers` (`utils/presentation/index.ts:91`), only drained at unmount — closed `AskSession`s are retained for the panel's lifetime. Detach per-turn in `runAsk`'s `finally`.
- **`DaemonClient.request` spread-order latent bug** (`utils/daemon/client.ts:100-107`): `...init` comes *after* `headers`, so a caller-supplied `init.headers` would clobber the merged auth headers — the inline comment claims the opposite. Harmless today (no caller passes headers).
- **Turbo double-mount**: `turbo:render` and `turbo:load` both fire per nav (`entrypoints/content/index.ts:61-62`), each doing `tearDown(); init();` → two mounts and two `POST /v1/sessions` per navigation.
- **Weak E2E assertion**: pairing.spec's invalid-code test (`e2e-browser/tests/pairing.spec.ts:123-125`) asserts `toHaveCount(0)` on the success text — passes instantly even if the click did nothing. Assert the visible error instead.
- **Bundle**: `content.js` is 197.6 kB (React DOM dominant); popup/options share a 147 kB vendor chunk. Preact via `preact/compat` would cut the content script to ~60 kB. Optional.

## Fix-verification

| Round-1 finding | Status | Evidence |
|---|---|---|
| C5 — snippet `dangerouslySetInnerHTML` | **Verified fixed** | `Popup.tsx:182-203` builds React text nodes + `<mark>`; no HTML parse path; `tests/popup-snippet.test.tsx` covers `<script>` payloads; bracket edge cases safe (cosmetic-only nesting quirk) |
| C6 — presentation effects orphaned on nav | **Verified fixed** | `QaPanel.tsx:43-55` cleanup runs `clearAll()`+`detachAll()`; reached on all three paths: panel close (`ContentApp.tsx:97` conditional render), Turbo nav and pagehide (`content/index.ts:57-63` → `root.unmount()`); `tests/qa-panel-cleanup.test.tsx` asserts page-DOM markers removed |
| C7 — `AskSession` stuck `inflight` | **Verified fixed** | `ws.ts:100-109` clears `inflight` in `settle()` unconditionally; `close()` idempotent (`ws.ts:183-195`); `tests/daemon-ws-inflight.test.ts` covers error-before-done, close-before-open, reuse. Only residual: a synchronous `wsFactory` throw (`ws.ts:93-95`) strands `inflight` until `close()` — unreachable via QaPanel's `finally` |
| I15 — pairing deep-link | **Verified fixed (with caveats)** | `Options.tsx:21-34,72-82`; unit test + real-daemon browser E2E (`pairing.spec.ts:26-71`). Caveats: no endpoint validation; link not clickable from web pages (above) |
| I16 — per-session mute | **Partially** | UI/persistence/a11y real (`QaPanel.tsx:101-111,312-324`), but end-to-end a no-op — see Important #1 |
| I18 — aria-live / dialog / focus trap | **Mostly fixed** | `aria-live="polite"` + `role="log"` (`QaPanel.tsx:384-389`); dialog semantics + Esc + shadow-aware Tab trap (`ExportModal.tsx:72-107`, uses `getRootNode().activeElement` — correct in shadow DOM); shift+Tab on first element redirects to last (`ExportModal.tsx:87-91`). Gaps: no focus restore to trigger on close; forward-Tab when focus has programmatically escaped the dialog is not recaptured (only the shift branch handles "active outside"); no shift+Tab unit test |
| I13 — Shell height/clamp/listeners | **Not fixed** | See Important #3 |
| I14 — Cmd-click document-wide | **Not fixed** | See Important #4 |
| I17 — open_link options | **Not fixed** | See Important #5 |

## Confirmed good

- **Browser E2E architecture** (`e2e-browser/`): per-test `mkdtempSync` profiles (no shared state), per-test daemon on port 0 with endpoint-file polling and 10 s deadline (`helpers/daemon.ts:200-219`), extension-id discovery handles the SW race (existing → `waitForEvent` → backgroundPages fallback → hard fail, `helpers/browser.ts:76-98`), `workers: 1` + action/navigation timeouts in `playwright.config.ts`. Specs assert real behavior: streamed `text_delta` text in the conversation, `highlight_lines` tagging actual fixture `<tr>`s in the *page* document plus footer counter plus clear-all round-trip (`presentation.spec.ts:136-169`), pairing verified down to `chrome.storage.local` contents.
- **Presentation handler safety** unchanged and still sound: `textContent` only, color/severity allowlists, `isSafeUrl` (`handlers.ts:220-228` — `&&`-precedence reads correctly: https anywhere, http only on 127.0.0.1, `//` rejected).
- **Frame validation, daemon client error normalization, storage typing, scrape defensiveness** — all hold up on re-read.
- `pnpm typecheck` clean; `pnpm test` 18 files / 60 tests green.

## Coverage notes

Read in full: all of `components/`, `utils/{daemon,github,presentation}/`, `utils/selection.ts` (skimmed), all `entrypoints/`, all 3 `e2e-browser/helpers/`, all 4 `e2e-browser/tests/`, `playwright.config.ts`, `package.json`, built `manifest.json`; tests read in full: `popup-snippet`, `qa-panel-cleanup`, `daemon-ws-inflight`, `options-deeplink`, `qa-panel-a11y-mute`. Greps: `mute_presentations` across `crates/` (zero hits), `collaps` across tests (zero hits), `open_link` wiring. Ran `pnpm typecheck` and `pnpm test` (both green). Did **not** run `pnpm test:browser` or `test:e2e` (requires cargo build; helpers and specs reviewed statically) and did not re-audit `styles.ts` or the node-E2E roundtrip beyond round-1 findings.
