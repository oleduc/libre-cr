// Verify the content script actually attaches to a "GitHub PR" page in a
// real Chromium and mounts its Shadow DOM host element. The fixture is
// served by intercepting `https://github.com/**` at the browser level — no
// real GitHub auth, no network.

import { test, expect } from "@playwright/test";
import { launchWithExtension } from "../helpers/browser";
import { FIXTURE_PR_URL, interceptGithub } from "../helpers/server";

test.describe("content script attach", () => {
  test("mounts #libre-cr-root on a fixture PR page", async () => {
    const browser = await launchWithExtension();
    try {
      await interceptGithub(browser.context);
      const page = await browser.context.newPage();
      await page.goto(FIXTURE_PR_URL);

      // The content script may take a moment to inject — `document_idle`
      // fires after DOMContentLoaded.
      const host = page.locator("#libre-cr-root");
      await expect(host).toHaveCount(1, { timeout: 5_000 });

      // Sanity: it's an actual host with a shadow root in the page DOM.
      const hasShadow = await host.evaluate(
        (el) => !!(el as HTMLElement).shadowRoot,
      );
      expect(hasShadow).toBe(true);

      // Sanity: the fixture's diff content is present (validates the
      // server-side route interception).
      await expect(page.locator('[data-tagsearch-path="src/auth.ts"]')).toHaveCount(1);
      await expect(page.locator('[data-tagsearch-path="src/users.ts"]')).toHaveCount(1);
    } finally {
      await browser.context.close();
    }
  });
});
