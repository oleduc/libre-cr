# Libre CR — User Guide

Libre CR is a code review companion that augments GitHub pull request pages with AI-assisted investigation. You — the reviewer — select code in the diff and ask focused questions; the assistant answers them grounded in your **actual local checkout** (full file contents, git history, cross-file references), not just the diff hunks visible in the browser. Everything except the LLM API call stays on your machine: conversation history, notes, and worktrees live in local SQLite and on local disk. The assistant never reviews autonomously — it only answers what you ask.

> Libre CR is pre-release software. It works end-to-end today, but you build it from source and load the extension unpacked. Package-manager installs and store-published extensions are planned.

## Contents

| Page | What's in it |
|---|---|
| [Installation](installation.md) | Prerequisites, building from source, loading the extension, verifying with `libre-cr doctor` |
| [Configuration](configuration.md) | Full `review.toml` / `code.toml` reference, providers (Anthropic, OpenAI-compatible, Ollama), mock mode, file locations |
| [Using Libre CR](using.md) | The review workflow: selections, verbs, notes, export, popup |
| [Troubleshooting](troubleshooting.md) | Symptom → cause → fix tables, logs, reset and uninstall |
| [FAQ](faq.md) | Privacy, data locations, multi-machine, MCP reuse |
| [Manual testing](../manual-testing.md) | Verify the whole stack by hand — incl. a keyless mock-provider smoke |

## Five-minute quick start

```sh
git clone https://github.com/libre-cr/libre-cr && cd libre-cr
cargo build --release                       # builds libre-cr, libre-cr-review, libre-cr-code
export PATH="$PWD/target/release:$PATH"     # or copy the three binaries somewhere on PATH
cd extension && pnpm install && pnpm build  # builds the extension to .output/chrome-mv3/
libre-cr start                              # runs in the foreground; keep this terminal open
```

1. **Load the extension**: in Chrome, open `chrome://extensions`, enable *Developer mode*, click *Load unpacked*, and pick `extension/.output/chrome-mv3/`.
2. **Pair**: in a second terminal run `libre-cr pair`, then open the extension's options page, paste the endpoint printed by `libre-cr start` (also in `~/.config/libre-cr/endpoint`) and the one-time code, and click *Pair*.
3. **Configure a provider**: run `libre-cr config` to open the config UI, pick `anthropic` or `openai_compat`, enter a model name and API key, and save. Changes apply immediately — no restart. (Or stay on the default `mock` provider for a key-less smoke test; see [Configuration](configuration.md#mock-provider-mode).)
4. **Open a GitHub PR** and click the floating Libre CR button. The daemon prepares a worktree for the PR head in the background.
5. **Select a line in the diff and click a verb** (e.g. *Explain*), or type a free-form question. The answer streams into the panel. Save conclusions as notes, then export them as a Markdown review draft.

If anything fails along the way, `libre-cr doctor` and `libre-cr logs` are your first stops — see [Troubleshooting](troubleshooting.md).
