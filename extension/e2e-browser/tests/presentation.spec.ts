// Drive a presentation effect round-trip: the daemon's mock provider emits
// a `highlight_lines` tool call, the panel's PresentationManager handles it
// and tags the matching `<tr>` in the fixture diff with
// `data-libre-cr-effect-id`. Clicking "Clear all" removes the markers.

import { test, expect } from "@playwright/test";
import { launchWithExtension } from "../helpers/browser";
import { startDaemon, type DaemonHandle } from "../helpers/daemon";
import { FIXTURE_PR_URL, interceptGithub } from "../helpers/server";

/**
 * Provider script that fires a `highlight_lines` tool-use on `src/auth.ts`
 * lines 10-11, then ends the turn. The agent dispatches it as a
 * `presentation_call` server frame.
 */
const HIGHLIGHT_SCRIPT = [
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "tool_use"',
  'id = "h1"',
  'name = "highlight_lines"',
  'input = { file = "src/auth.ts", start_line = 10, end_line = 11, color = "yellow", label = "watch this" }',
  "",
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "done"',
  "input_tokens = 1",
  "output_tokens = 1",
  'stop_reason = "tool_use"',
  "",
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "text_delta"',
  'text = "Highlighted."',
  "",
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "done"',
  "input_tokens = 1",
  "output_tokens = 1",
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

test.describe("presentation tools", () => {
  let daemon: DaemonHandle;

  test.beforeEach(async () => {
    daemon = await startDaemon({ scriptBlock: HIGHLIGHT_SCRIPT });
  });

  test.afterEach(async () => {
    await daemon?.kill();
  });

  test("highlight_lines tags fixture rows; Clear all removes them", async ({}, testInfo) => {
    // The clear-all assertion needs DOM stability; bump the test budget.
    testInfo.setTimeout(45_000);
    const browser = await launchWithExtension();
    try {
      await injectAuth(browser, daemon);
      await interceptGithub(browser.context);

      const page = await browser.context.newPage();
      await page.goto(FIXTURE_PR_URL);
      const host = page.locator("#libre-cr-root");
      await expect(host).toHaveCount(1, { timeout: 5_000 });
      // Fresh sessions start closed behind the CR button; open the panel.
      await host.evaluate((el) => {
        (el as HTMLElement).shadowRoot
          ?.querySelector<HTMLButtonElement>(".libre-cr-reopen")
          ?.click();
      });

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

      // Ask anything — the mock script ignores the input and fires its
      // pre-baked highlight_lines tool call.
      await host.evaluate((el) => {
        const sr = (el as HTMLElement).shadowRoot;
        const ta = sr?.querySelector<HTMLTextAreaElement>("textarea");
        const setter = Object.getOwnPropertyDescriptor(
          HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        setter?.call(ta, "highlight 10..11 please");
        ta?.dispatchEvent(new Event("input", { bubbles: true }));
        const buttons = Array.from(
          sr?.querySelectorAll<HTMLButtonElement>("button") ?? [],
        );
        const ask = buttons.find((b) => b.textContent?.includes("Ask"));
        ask?.click();
      });

      // Effect lands on the real <tr> in the fixture diff. We look at the
      // PAGE document, not the shadow root — `highlight_lines` decorates
      // the live diff, not the panel UI.
      const taggedRow = page.locator(
        '[data-tagsearch-path="src/auth.ts"] tr[data-libre-cr-effect-id][data-libre-cr-tag="highlight"]',
      );
      await expect(taggedRow).toHaveCount(2, { timeout: 10_000 });

      // The tagged rows should also pick up the color class.
      const colorClassed = await page.locator(
        '[data-tagsearch-path="src/auth.ts"] tr.libre-cr-hl-yellow',
      ).count();
      expect(colorClassed).toBe(2);

      // Footer counter reflects the effect.
      await expect
        .poll(
          async () =>
            host.evaluate((el) => {
              const sr = (el as HTMLElement).shadowRoot;
              return sr?.querySelector(".libre-cr-footer")?.textContent ?? "";
            }),
          { timeout: 5_000 },
        )
        .toContain("1 highlight");

      // Click "Clear all" in the footer; rows lose their markers.
      await host.evaluate((el) => {
        const sr = (el as HTMLElement).shadowRoot;
        const buttons = Array.from(
          sr?.querySelectorAll<HTMLButtonElement>(".libre-cr-footer button") ?? [],
        );
        const clear = buttons.find((b) => b.textContent?.includes("Clear highlights"));
        clear?.click();
      });

      await expect(taggedRow).toHaveCount(0, { timeout: 5_000 });
    } finally {
      await browser.context.close();
    }
  });
});
