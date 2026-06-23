// Background service worker.
//
// Phase 0 scaffolding: minimal. The content script talks to the local daemon
// directly via HTTP/WS (per `05-browser-extension.md` § Transport from a
// Content Script). The service worker stays available as a fallback proxy if
// future browser changes block content-script localhost calls.

export default defineBackground(() => {
  // Intentionally empty. Filled in later phases.
});
