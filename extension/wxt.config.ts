import { defineConfig } from "wxt";

export default defineConfig({
  modules: ["@wxt-dev/module-react"],
  manifest: {
    name: "Libre CR",
    description:
      "Code review companion: ask focused questions about a GitHub PR, grounded in your local checkout.",
    permissions: ["storage"],
    host_permissions: ["*://github.com/*", "http://127.0.0.1/*", "http://localhost/*"],
    action: { default_popup: "popup.html" },
    options_ui: { page: "options.html", open_in_tab: true },
  },
});
