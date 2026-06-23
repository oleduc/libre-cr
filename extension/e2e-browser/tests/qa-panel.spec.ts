// Drive the Q&A panel end-to-end: pair, navigate to a fixture PR page,
// observe verbs streaming in, ask a question, and watch text deltas appear
// in the conversation pane.
//
// We inject auth directly into `chrome.storage.local` via the extension's
// service worker instead of routing through the UI. The pairing UI flow is
// already covered by `pairing.spec.ts`; here we want a stable starting
// point for the panel-side assertions.

import { test, expect } from "@playwright/test";
import { launchWithExtension } from "../helpers/browser";
import { startDaemon, type DaemonHandle } from "../helpers/daemon";
import { FIXTURE_PR_URL, interceptGithub } from "../helpers/server";

/** Inline mock provider script: one `text_delta` then a `done`. */
const TEXT_DELTA_SCRIPT = [
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "text_delta"',
  'text = "Reviewed: looks fine."',
  "",
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "done"',
  "input_tokens = 1",
  "output_tokens = 2",
  'stop_reason = "end_turn"',
  "",
].join("\n");

async function injectAuth(
  browser: Awaited<ReturnType<typeof launchWithExtension>>,
  daemon: DaemonHandle,
): Promise<void> {
  const sw = browser.context.serviceWorkers()[0];
  if (!sw) throw new Error("background service worker did not start");
  await sw.evaluate(
    async (auth: { endpoint: string; token: string; origin: string }) => {
      await (globalThis as unknown as {
        chrome: {
          storage: { local: { set: (items: Record<string, unknown>) => Promise<void> } };
        };
      }).chrome.storage.local.set({
        "daemon.endpoint": auth.endpoint,
        "daemon.token": auth.token,
        "daemon.extension_origin": auth.origin,
      });
    },
    {
      endpoint: daemon.endpoint,
      token: daemon.token,
      origin: `chrome-extension://${browser.extensionId}`,
    },
  );
}

test.describe("Q&A panel", () => {
  let daemon: DaemonHandle;

  test.beforeEach(async () => {
    daemon = await startDaemon({ scriptBlock: TEXT_DELTA_SCRIPT });
  });

  test.afterEach(async () => {
    await daemon?.kill();
  });

  test("panel reaches ready and loads verbs from /v1/verbs", async () => {
    const browser = await launchWithExtension();
    try {
      await injectAuth(browser, daemon);
      await interceptGithub(browser.context);

      const page = await browser.context.newPage();
      await page.goto(FIXTURE_PR_URL);
      const host = page.locator("#libre-cr-root");
      await expect(host).toHaveCount(1, { timeout: 5_000 });

      // Wait for the panel to reach `ready` state (verbs bucket rendered).
      await expect
        .poll(
          async () =>
            host.evaluate((el) => {
              const sr = (el as HTMLElement).shadowRoot;
              return !!sr?.querySelector('[data-testid="verbs"]');
            }),
          { timeout: 10_000 },
        )
        .toBe(true);

      // The verbs container must contain the daemon's `/v1/verbs` payload.
      // The mock provider returns five verbs (per `09-presentation-tools.md`).
      const verbCount = await host.evaluate((el) => {
        const sr = (el as HTMLElement).shadowRoot;
        const verbs = sr?.querySelector('[data-testid="verbs"]');
        // Filter to buttons only — the container also holds a hint span.
        return verbs?.querySelectorAll("button").length ?? 0;
      });
      expect(verbCount).toBeGreaterThanOrEqual(5);
    } finally {
      await browser.context.close();
    }
  });

  test("asking a question streams text into the conversation", async () => {
    const browser = await launchWithExtension();
    try {
      await injectAuth(browser, daemon);
      await interceptGithub(browser.context);

      const page = await browser.context.newPage();
      await page.goto(FIXTURE_PR_URL);
      const host = page.locator("#libre-cr-root");
      await expect(host).toHaveCount(1, { timeout: 5_000 });

      // Wait for ready.
      await expect
        .poll(
          async () =>
            host.evaluate((el) => {
              const sr = (el as HTMLElement).shadowRoot;
              return !!sr?.querySelector('[data-testid="verbs"]');
            }),
          { timeout: 10_000 },
        )
        .toBe(true);

      // Type a question into the panel's textarea (inside the shadow root).
      await host.evaluate((el) => {
        const sr = (el as HTMLElement).shadowRoot;
        const ta = sr?.querySelector<HTMLTextAreaElement>("textarea");
        if (!ta) throw new Error("textarea missing");
        ta.focus();
        // React listens to onChange — set via native setter so the event fires.
        const setter = Object.getOwnPropertyDescriptor(
          HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        setter?.call(ta, "Anything to note here?");
        ta.dispatchEvent(new Event("input", { bubbles: true }));
      });

      // Click "Ask".
      await host.evaluate((el) => {
        const sr = (el as HTMLElement).shadowRoot;
        const buttons = Array.from(sr?.querySelectorAll<HTMLButtonElement>("button") ?? []);
        const ask = buttons.find((b) => b.textContent?.includes("Ask"));
        if (!ask) throw new Error("Ask button missing");
        ask.click();
      });

      // Wait for the mock provider's text_delta to land in the conversation.
      await expect
        .poll(
          async () =>
            host.evaluate((el) => {
              const sr = (el as HTMLElement).shadowRoot;
              const conv = sr?.querySelector(".libre-cr-conversation");
              return conv?.textContent ?? "";
            }),
          { timeout: 10_000 },
        )
        .toContain("Reviewed: looks fine.");
    } finally {
      await browser.context.close();
    }
  });
});
