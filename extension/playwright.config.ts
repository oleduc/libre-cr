// Playwright config for browser-driven E2E tests.
//
// These tests load the built extension (`.output/chrome-mv3`) into a real
// Chromium and drive it against:
//   - a Playwright `page.route()` interception that serves a fixture HTML
//     file at `https://github.com/foo/bar/pull/1` (so the manifest's
//     `*://github.com/*/pull/*` match fires without requiring real GitHub),
//   - a freshly-spawned `libre-cr-review` daemon configured with the mock
//     provider so the WS round-trip is deterministic.
//
// Run with `pnpm test:browser`. The script does `wxt build &&` first so the
// extension files exist on disk at the path Chromium expects.

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e-browser/tests",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  // Extension loading is heavy and the spawned daemons listen on real ports;
  // serializing the suite keeps things deterministic.
  workers: 1,
  fullyParallel: false,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["github"]] : [["list"]],
  use: {
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    // All page interactions default to a sensible bound — a misbehaving test
    // fails fast instead of hanging the suite.
    actionTimeout: 5_000,
    navigationTimeout: 10_000,
  },
});
