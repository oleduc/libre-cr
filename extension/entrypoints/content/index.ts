// Content script — runs on GitHub PR pages.
// Implements the lifecycle from `specs/05-browser-extension.md`
// § Content Script Lifecycle.

import { createRoot, type Root } from "react-dom/client";
import { createElement } from "react";

import { ContentApp } from "../../components/ContentApp";
import { isPullRequestPage } from "../../utils/github/detect";

export default defineContentScript({
  matches: ["*://github.com/*/pull/*"],
  runAt: "document_idle",
  main() {
    let root: Root | null = null;
    let host: HTMLElement | null = null;

    const tearDown = () => {
      try {
        root?.unmount();
      } catch {
        // ignore
      }
      root = null;
      if (host?.parentNode) host.parentNode.removeChild(host);
      host = null;
    };

    const init = () => {
      const pr = isPullRequestPage();
      if (!pr) {
        tearDown();
        return;
      }
      if (root) return; // already mounted
      const prUrl = `https://github.com/${pr.owner}/${pr.repo}/pull/${pr.number}`;
      host = document.createElement("div");
      host.id = "libre-cr-root";
      // Avoid being affected by GitHub's CSS.
      host.style.all = "initial";
      const shadow = host.attachShadow({ mode: "open" });
      const styleEl = document.createElement("style");
      // Inject the styles. Lazy-loaded via dynamic import would split chunks;
      // a static import keeps the bundle simple.
      // eslint-disable-next-line @typescript-eslint/no-require-imports
      shadow.appendChild(styleEl);
      const mount = document.createElement("div");
      shadow.appendChild(mount);
      document.documentElement.appendChild(host);
      root = createRoot(mount);
      root.render(createElement(ContentApp, { prUrl, styleEl }));
    };

    init();

    // GitHub uses Turbo; listen for nav events.
    const onTurbo = () => {
      tearDown();
      init();
    };
    document.addEventListener("turbo:render", onTurbo);
    document.addEventListener("turbo:load", onTurbo);
    window.addEventListener("pagehide", tearDown);
  },
});
