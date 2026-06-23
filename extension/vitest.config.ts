import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: false,
    include: ["tests/**/*.test.ts", "tests/**/*.test.tsx"],
    // Unit / component tests only. E2E and browser-driven suites live under
    // `extension/e2e/` and `extension/e2e-browser/` respectively, with their
    // own configs and `pnpm test:e2e` / `pnpm test:browser` scripts.
    exclude: ["node_modules/**", "e2e/**", "e2e-browser/**"],
    setupFiles: ["tests/setup.ts"],
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./", import.meta.url)),
    },
  },
});
