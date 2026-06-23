// Browser launch helper for browser-driven E2E tests.
//
// Loads the built MV3 extension (`.output/chrome-mv3`) via the canonical
// Chromium flag combination: `--disable-extensions-except=<path>` plus
// `--load-extension=<path>`. We use `launchPersistentContext` because MV3
// service workers require a persistent profile.

import { chromium, type BrowserContext } from "@playwright/test";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const EXT_ROOT = resolve(HERE, "../..");
export const BUILT_EXT_PATH = resolve(EXT_ROOT, ".output/chrome-mv3");

export interface LaunchedBrowser {
  context: BrowserContext;
  /** The extension's chrome-extension://<id> ID, discovered from the SW. */
  extensionId: string;
  userDataDir: string;
}

/**
 * Launch a fresh, persistent Chromium context with the extension loaded.
 *
 * The `extensionId` is discovered by waiting for the MV3 background service
 * worker URL (`chrome-extension://<id>/background.js`). This is the standard
 * trick — it works in headless and headed alike.
 */
export async function launchWithExtension(options: {
  headless?: boolean;
} = {}): Promise<LaunchedBrowser> {
  if (!existsSync(BUILT_EXT_PATH)) {
    throw new Error(
      `Built extension not found at ${BUILT_EXT_PATH}. Did 'wxt build' run? ` +
        `'pnpm test:browser' chains the build first.`,
    );
  }
  const userDataDir = mkdtempSync(join(tmpdir(), "lcr-browser-profile-"));
  const headless =
    options.headless ?? !(process.env.PWDEBUG || process.env.HEADED);

  // The default `chromium` channel resolves to the `headless_shell` binary,
  // which does not load extensions. We pin `channel: "chromium"` so Playwright
  // uses the full Chromium / Chrome-for-Testing binary, and pass
  // `--headless=new` when headless — that mode supports MV3 service workers.
  const context = await chromium.launchPersistentContext(userDataDir, {
    headless: false, // bypass headless_shell; we add --headless=new ourselves
    channel: "chromium",
    args: [
      ...(headless ? ["--headless=new"] : []),
      `--disable-extensions-except=${BUILT_EXT_PATH}`,
      `--load-extension=${BUILT_EXT_PATH}`,
      "--no-first-run",
      "--no-default-browser-check",
      // The extension on a real GitHub page wouldn't trip this — installed
      // extensions are exempt from Private Network Access. But Playwright's
      // `--load-extension=` loads it as unpacked / external, so Chrome
      // enforces PNA. Disabling the feature lets the content script reach
      // the daemon at 127.0.0.1 from the (intercepted) https://github.com
      // origin, matching what the production extension experiences.
      "--disable-features=PrivateNetworkAccessSendPreflights,BlockInsecurePrivateNetworkRequests,PrivateNetworkAccessRespectPreflightResults,LocalNetworkAccessChecks",
    ],
  });

  // Wait for the SW (or worker) to come up so we can extract the extension id.
  // The SW may already exist (race), so we poll first; if not, wait for the
  // `serviceworker` event. Use Promise.race to handle both timing patterns.
  const matchId = (url: string): string => {
    const m = url.match(/^chrome-extension:\/\/([a-z]+)\//);
    return m?.[1] ?? "";
  };

  let extensionId = "";
  const existingSw = context.serviceWorkers()[0];
  if (existingSw) {
    extensionId = matchId(existingSw.url());
  }
  if (!extensionId) {
    try {
      const sw = await context.waitForEvent("serviceworker", { timeout: 10_000 });
      extensionId = matchId(sw.url());
    } catch {
      // fall through to the polling fallback below for the MV2 case.
    }
  }
  if (!extensionId) {
    const bgPages = context.backgroundPages?.() ?? [];
    if (bgPages[0]) extensionId = matchId(bgPages[0].url());
  }
  if (!extensionId) {
    await context.close();
    throw new Error(
      "Failed to detect extension id from the service worker URL within 10s.",
    );
  }

  return { context, extensionId, userDataDir };
}
