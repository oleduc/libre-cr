// Vitest config for end-to-end tests that spawn the real Rust binaries and
// drive them with the extension's own daemon-client code. These tests need
// a Node environment (for `child_process`, `fetch`, `WebSocket`) — the
// default jsdom environment doesn't host real network sockets cleanly.
//
// Run with: `pnpm test:e2e` (see package.json).
import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
  test: {
    environment: "node",
    globals: false,
    include: ["e2e/**/*.test.ts"],
    // Spawning + lazy-building the Rust binaries on first contact can take a
    // while on a cold cache. The individual tests still bound their reads.
    testTimeout: 30_000,
    hookTimeout: 60_000,
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./", import.meta.url)),
    },
  },
});
