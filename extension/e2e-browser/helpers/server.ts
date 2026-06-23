// Fixture page interception helper.
//
// We don't run an actual HTTP(S) server. Instead, we use Playwright's
// `BrowserContext.route()` to fulfill requests to `https://github.com/**`
// with the fixture HTML/CSS that ship in `e2e-browser/fixtures/`. This is
// the cleanest way to satisfy the extension manifest's
// `matches: ["*://github.com/*/pull/*"]` content-script pattern without
// touching the build.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { BrowserContext } from "@playwright/test";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURES = resolve(HERE, "../fixtures");

export const FIXTURE_PR_URL = "https://github.com/foo/bar/pull/1";

/**
 * Install request interception so navigating to a `https://github.com/.../pull/...`
 * URL serves the fixture page, and `<link href="/__fixture__/pr-page.css">`
 * loads the local stylesheet. Anything else under github.com 404s — tests
 * that need a real subresource should add their own routes.
 */
export async function interceptGithub(context: BrowserContext): Promise<void> {
  await context.route("https://github.com/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/__fixture__/pr-page.css") {
      const body = readFileSync(resolve(FIXTURES, "pr-page.css"), "utf8");
      await route.fulfill({
        status: 200,
        headers: { "content-type": "text/css; charset=utf-8" },
        body,
      });
      return;
    }
    if (/^\/[^/]+\/[^/]+\/pull\/\d+\/?$/.test(url.pathname)) {
      const body = readFileSync(resolve(FIXTURES, "pr-page.html"), "utf8");
      await route.fulfill({
        status: 200,
        headers: { "content-type": "text/html; charset=utf-8" },
        body,
      });
      return;
    }
    // Default — return an empty 404 so we don't accidentally touch the
    // real internet during a test run.
    await route.fulfill({ status: 404, body: "fixture not handled" });
  });
}
