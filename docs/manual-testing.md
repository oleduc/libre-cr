# Manual Testing Guide

How to exercise Libre CR by hand, from a fast no-setup sanity check to a full
real-provider run. Three tiers, fastest first — do as many as the change you're
testing warrants.

This guide is a walkthrough; it links out to the reference docs rather than
duplicating them:

- [User guide](user-guide/README.md) — [installation](user-guide/installation.md), [configuration](user-guide/configuration.md), [daily workflow](user-guide/using.md), [troubleshooting](user-guide/troubleshooting.md)
- [Developer docs](development/README.md) — [setup](development/setup.md), [testing](development/testing.md) (the authoritative suite/command reference)

Prerequisites for any live tier: `git`, Chrome ≥ 120 (or Firefox ≥ 121), a Rust
toolchain, and Node 20 + pnpm 9. An LLM API key is **only** needed for Tier 3.

---

## Tier 1 — Automated suites (≈2 min, no setup)

The fastest confidence check. No API key, no browser interaction, no daemon to
start. Run this before and after any change.

```sh
export PATH="$HOME/.cargo/bin:$PATH"

# Rust — all crates, incl. the spawn-real-binary E2E suites in libre-cr-e2e
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Extension — unit + component
cd extension && pnpm install && pnpm test

# Extension E2E — spawns the real daemons, drives them with the extension's
# own client code
pnpm test:e2e

# Browser E2E — loads the built extension into real Chromium
pnpm exec playwright install chromium   # one time
pnpm test:browser
```

`cargo test --workspace` includes the `libre-cr-e2e` crate, which builds and
spawns the real `libre-cr-code` / `libre-cr-review` binaries and talks to them
over the wire (MCP stdio, HTTP/WS). Set `LIBRE_CR_E2E_REQUIRED=1` to turn a
missing-binary skip into a hard failure (CI does this). Full taxonomy, single-test
invocation, and the CI matrix: [development/testing.md](development/testing.md).

---

## Tier 2 — Keyless full-stack smoke (mock provider)

This proves the entire pipeline — daemon supervision, pairing, the extension
transport, the Q&A panel, presentation effects — **without an API key and
without cloning the PR's repo**. The mock provider replays a scripted answer,
and `code_intel = true` hands each session a fake worktree instantly, so you're
testing the plumbing in isolation from the LLM and from git.

### 1. Build

```sh
cargo build --release
cd extension && pnpm build && cd ..
export PATH="$PWD/target/release:$PATH"
```

### 2. Write a mock config

Put this in `~/.config/libre-cr/review.toml`. Note the exact field names — the
script is `[[mock.provider_script]]` and each entry's `event` is an inline table
whose `type` is the serde tag (snake_case):

```toml
[provider]
kind = "mock"
model = "mock"

[mock]
code_intel = true                     # sessions get a fake worktree immediately

[[mock.provider_script]]
event = { type = "text_delta", text = "Reviewed: looks fine." }

[[mock.provider_script]]
event = { type = "done", input_tokens = 1, output_tokens = 2, stop_reason = "end_turn" }
```

You can add a `delay_ms = <n>` to any script entry to simulate streaming latency.

### 3. Start the daemons

`libre-cr start` runs the supervisor in the **foreground** — leave it running
and use a second terminal for the rest.

```sh
libre-cr start
```

### 4. Load the extension

`chrome://extensions` → enable **Developer mode** → **Load unpacked** →
select `extension/.output/chrome-mv3/`. Note the extension ID Chrome assigns.

### 5. Pair

```sh
libre-cr pair          # prints a one-time code, issued through the running daemon
libre-cr status        # shows the endpoint URL
```

Click the Libre CR toolbar icon → **Pair extension**, paste the endpoint and the
code. Pairing auto-persists your extension origin, so no manual CORS edit is
needed. (Details and the deep-link alternative: [installation §4](user-guide/installation.md).)

### 6. Exercise it

Open any GitHub PR, click the floating **CR** button, select a line, and ask
anything. You get the canned `Reviewed: looks fine.` This confirms the full
chain: scrape → pair → session → (fake) worktree → agent loop → stream → render.

**What to click to cover the surface:** the five verbs, a free-form question,
the thinking-trace expander, the 🔇 mute toggle (muted → no presentation
effects apply), save-as-note with a severity, and the export modal.

---

## Tier 3 — Full real run (with a provider)

Same as Tier 2, but with a real LLM backend and a real repo fetch. Skip the mock
`review.toml` — configure the provider through the UI instead.

### Configure a provider

```sh
libre-cr config        # opens http://127.0.0.1:<port>/config-ui?token=…
```

In the config UI: pick a provider, then **Fetch models** to populate the model
dropdown from the provider's live API, and save. Provider changes **hot-reload —
no restart needed.**

Two credential paths (see [configuration.md](user-guide/configuration.md) for
the full reference):

- **API key** — `anthropic` or `openai_compat`; paste the key in the config UI
  (it's stored encrypted; you can't hand-edit it into `review.toml`).
- **Environment variable** — set `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` before
  `libre-cr start`; the config UI shows a ✓ when it detects one, and you can
  leave the key field blank to use it.

### Run a review

On a real PR — pick a **small public** one first so the worktree fetch is fast.
The daemon does a real `git fetch` of the PR ref into a managed worktree, so:

- **Public repo** → just works.
- **Private repo** → your local `git` needs working credentials (SSH key /
  credential helper) for that org.

Then work through the real workflow: verbs against real code, follow-up
questions, presentation highlights landing on the actual diff, notes, and export
→ paste into GitHub's review composer. Full workflow: [using.md](user-guide/using.md).

---

## Observability while testing

```sh
libre-cr status        # endpoint, PIDs, /v1/health probe
libre-cr logs -f       # tail both daemons live (review + code)
libre-cr logs -n 200   # last N lines instead of following
libre-cr doctor        # git present? token perms? binaries on PATH?
```

Logs live in `~/.local/state/libre-cr/log/`:

- `libre-cr-review.log` — agent loop, HTTP requests, provider calls
- `libre-cr-code.log` — MCP traffic, git operations, worktree events
- `supervisor.log` — start/stop/restart events

## Reset

```sh
libre-cr stop
rm ~/.config/libre-cr/{token,endpoint}
libre-cr start
libre-cr pair          # re-pair the extension
```

More recovery procedures (worktree-pending, provider errors, selector breakage,
uninstall) are in [troubleshooting.md](user-guide/troubleshooting.md).

---

## Which tier for which change?

| You changed… | Run at least |
|---|---|
| Rust logic, a tool, a provider parser | Tier 1 |
| Wire format (routes, WS frames, MCP tools) | Tier 1 + Tier 2 (both sides talk) |
| Extension UI, panel, presentation effects | Tier 2 |
| Provider integration, agent-loop behavior on real models | Tier 3 |
| Anything you're about to ship | all three |
