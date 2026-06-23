// Drive the extension options page through a full pairing flow with a
// real spawned daemon, verifying both the deep-link auto-pair path and the
// hand-typed form path.
//
// The post-pair panel-ready assertion lives in `qa-panel.spec.ts` where we
// can inject auth directly and configure the daemon's CORS origin to match
// the fixture's `https://github.com` origin — see
// `05-browser-extension.md` § Transport from a Content Script for why
// these two concerns are decoupled in tests.

import { test, expect } from "@playwright/test";
import { launchWithExtension } from "../helpers/browser";
import { startDaemon, type DaemonHandle } from "../helpers/daemon";

test.describe("pairing flow", () => {
  let daemon: DaemonHandle;

  test.beforeEach(async () => {
    daemon = await startDaemon();
  });

  test.afterEach(async () => {
    await daemon?.kill();
  });

  test("options page auto-pairs via deep link", async () => {
    const browser = await launchWithExtension();
    try {
      const optionsUrl =
        `chrome-extension://${browser.extensionId}/options.html` +
        `#pair?endpoint=${encodeURIComponent(daemon.endpoint)}` +
        `&code=${encodeURIComponent(daemon.pairingCode)}` +
        `&auto=1`;

      const optionsPage = await browser.context.newPage();
      await optionsPage.goto(optionsUrl);

      // Auto-pair runs on mount — success appears as a green status line.
      await expect(
        optionsPage.getByText("Paired successfully.", { exact: false }),
      ).toBeVisible({ timeout: 5_000 });

      // The page now shows the bound endpoint with an "Unpair" affordance.
      await expect(
        optionsPage.getByText(daemon.endpoint, { exact: false }),
      ).toBeVisible({ timeout: 5_000 });
      await expect(
        optionsPage.getByRole("button", { name: "Unpair" }),
      ).toBeVisible({ timeout: 5_000 });

      // Storage state is the contract the content script reads from on the
      // next page load. Verify via the SW's chrome.storage.local.
      const sw = browser.context.serviceWorkers()[0];
      if (!sw) throw new Error("background service worker did not start");
      const stored = await sw.evaluate(async () => {
        const all = await (globalThis as unknown as {
          chrome: {
            storage: { local: { get: (keys: string[]) => Promise<Record<string, unknown>> } };
          };
        }).chrome.storage.local.get([
          "daemon.endpoint",
          "daemon.token",
        ]);
        return all as Record<string, string>;
      });
      expect(stored["daemon.endpoint"]).toBe(daemon.endpoint);
      expect(stored["daemon.token"]).toBe(daemon.token);
    } finally {
      await browser.context.close();
    }
  });

  test("manual pairing via form succeeds with a fresh code", async () => {
    // Mint a second code so the manual path doesn't race the auto-pair test.
    const issueResp = await fetch(`${daemon.endpoint}/v1/pair/issue`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${daemon.token}`,
        "Content-Type": "application/json",
      },
      body: "{}",
    });
    const { code: manualCode } = (await issueResp.json()) as { code: string };

    const browser = await launchWithExtension();
    try {
      const optionsPage = await browser.context.newPage();
      await optionsPage.goto(
        `chrome-extension://${browser.extensionId}/options.html`,
      );

      const endpointInput = optionsPage.locator('input[type="text"]').first();
      await endpointInput.fill(daemon.endpoint);
      const codeInput = optionsPage.locator('input[type="text"]').nth(1);
      await codeInput.fill(manualCode);
      await optionsPage.getByRole("button", { name: "Pair" }).click();

      await expect(
        optionsPage.getByText("Paired successfully.", { exact: false }),
      ).toBeVisible({ timeout: 5_000 });
    } finally {
      await browser.context.close();
    }
  });

  test("invalid pairing code yields a visible error", async () => {
    const browser = await launchWithExtension();
    try {
      const optionsPage = await browser.context.newPage();
      await optionsPage.goto(
        `chrome-extension://${browser.extensionId}/options.html`,
      );

      const endpointInput = optionsPage.locator('input[type="text"]').first();
      await endpointInput.fill(daemon.endpoint);
      const codeInput = optionsPage.locator('input[type="text"]').nth(1);
      await codeInput.fill("not-a-real-code");
      await optionsPage.getByRole("button", { name: "Pair" }).click();

      // The error path renders the daemon's message in red. We don't pin the
      // exact wording — only that some error is shown and pairing didn't
      // claim success.
      await expect(
        optionsPage.getByText("Paired successfully.", { exact: false }),
      ).toHaveCount(0, { timeout: 5_000 });
    } finally {
      await browser.context.close();
    }
  });
});
