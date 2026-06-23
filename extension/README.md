# Libre CR — Browser Extension

The Manifest V3 extension that augments GitHub PR pages with selection UI,
a Q&A panel, and presentation effects. The extension is intentionally thin:
the daemon is the source of truth for all conversation and code data.

> Status: Phase 0 (scaffolding). See [`../specs/05-browser-extension.md`](../specs/05-browser-extension.md).

## Develop

```sh
pnpm install
pnpm dev
```

`pnpm dev` runs WXT in watch mode and launches a Chrome profile with the
extension loaded. `pnpm dev:firefox` does the same for Firefox.

## Build

```sh
pnpm build           # Chrome
pnpm build:firefox   # Firefox
pnpm zip             # zipped artifact for the Web Store
```

## Layout

- `entrypoints/background.ts` — service worker.
- `entrypoints/content/` — content script that runs on PR pages.
- `entrypoints/popup/` — toolbar popup.
- `entrypoints/options/` — options page.

POC port lands in Phase 5: GitHub adapter/selectors, Shadow DOM shell,
theme detection, floating widget mechanics, EventBus, CSP-safe validator.
