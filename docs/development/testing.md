# Testing

Seven suites across two languages. The rule of thumb for *where a new test
goes*: test at the lowest layer that can actually catch the bug.

## Rust suites

| Suite | Where | Runs | What it covers |
|---|---|---|---|
| Unit | `#[cfg(test)]` modules beside the code, all crates | in-process | parsers, stores, frame round-trips, supervisor policy, registries |
| Integration | `crates/*/tests/` (e.g. `libre-cr-review/tests/http_api.rs`, `ws_smoke.rs`, `verbs_api.rs`; `libre-cr-code/tests/mcp_stdio.rs`; `libre-cr-cli/tests/integration.rs`) | in-process server / spawned single binary | HTTP routes against the real axum app, WS round-trips with mock provider, MCP framing |
| E2E: `mcp_consumer` (21 tests) | `crates/libre-cr-e2e/tests/mcp_consumer.rs` | spawns real `libre-cr-code` | the full MCP consumer contract: handshake, tool schemas, every tool against a real git fixture repo |
| E2E: `http_consumer` (21 tests) | `crates/libre-cr-e2e/tests/http_consumer.rs` | spawns real `libre-cr-review` (+ `libre-cr-code`) | the extension's view: pairing, sessions, asks over WS, scripted provider streams, presentation frames |
| E2E: `spawned_smoke` (3 tests) | `crates/libre-cr-e2e/tests/spawned_smoke.rs` | spawns both binaries | supervision/restart behavior, daemon lifecycle |

```sh
cargo test --workspace                 # everything (CI runs this with --all-features)
cargo test -p libre-cr-review          # one crate
cargo test -p libre-cr-e2e --test http_consumer        # one E2E suite
cargo test -p libre-cr-review --test http_api          # one integration file
cargo test -p libre-cr-review busy_guard               # filter by test name
cargo test -p libre-cr-e2e --test mcp_consumer grep -- --nocapture   # see daemon stderr
```

## Extension suites

| Suite | Where | Runs | What it covers |
|---|---|---|---|
| Unit (74 tests) | `extension/tests/*.test.ts(x)` | vitest + jsdom | components, frame parsing, scrape/diff/detect, presentation handlers, storage |
| Node E2E (5 tests) | `extension/e2e/daemon-roundtrip.test.ts` | vitest (node env) against a spawned real daemon | the extension's `DaemonClient`/`AskSession` talking to a real `libre-cr-review` over the wire |
| Browser E2E (7 tests) | `extension/e2e-browser/tests/*.spec.ts` | Playwright + real Chromium with the built extension loaded | content-script attach, pairing UI, Q&A panel, presentation effects — full stack |

```sh
cd extension
pnpm test                              # unit suite (~6 s)
pnpm test tests/qa-panel.test.tsx      # one file
pnpm test -- -t "collapse"             # filter by test name
pnpm test:e2e                          # node E2E (lazily cargo-builds the daemons)
pnpm test:browser                      # wxt build + playwright (lazily cargo-builds too)
pnpm test:browser:headed               # same, headed — best way to debug
```

## How the spawn-binary harnesses work

All three harness families share the same pattern (Rust:
`crates/libre-cr-e2e/tests/common/{spawned_daemon,mcp_client,git_fixture}.rs`;
Node: `extension/e2e/daemon-roundtrip.test.ts`; Playwright:
`extension/e2e-browser/helpers/{daemon,server,browser}.ts`):

1. **Lazy cargo build, once per test process** — `cargo build -p
   libre-cr-review --bin libre-cr-review` (and `-code`) behind a `Once` /
   memoized promise. If you already built, this is a no-op stat.
2. **Tempdir `$HOME`** — each spawned daemon gets a fresh temp home, so its
   `~/.config/libre-cr/` and `state.db` are isolated and disposable. A
   `review.toml` is written there configuring the **mock provider** (often
   with a scripted event stream) and a known token.
3. **Ephemeral ports** — `server.port = 0`; the test discovers the real URL
   by polling the `endpoint` file with a deadline (no sleeps-as-sync).
4. **Drop/teardown kills the child**; the tempdir keeps the daemon's home
   alive for the handle's lifetime.

### `LIBRE_CR_E2E_REQUIRED`

If the lazy build fails (no Rust toolchain, broken source), the suites
**skip** rather than fail — correct for someone hacking only on the
extension. `LIBRE_CR_E2E_REQUIRED=1` turns every skip into a hard failure;
CI sets it (rust + browser jobs in `.github/workflows/ci.yml`) so a broken
Rust build can never show green E2E.

```sh
LIBRE_CR_E2E_REQUIRED=1 cargo test -p libre-cr-e2e    # run it like CI does
```

## The Playwright suite, specifically

`extension/playwright.config.ts` + `e2e-browser/helpers/browser.ts` encode
three non-obvious tricks — don't fight them:

- **Chromium channel requirement.** Playwright's default `chromium` resolves
  to `headless_shell`, which cannot load extensions. The helper pins
  `channel: "chromium"` (full Chromium / Chrome-for-Testing) and passes
  `--headless=new` itself. First-time setup:
  `pnpm exec playwright install --with-deps chromium`.
- **github.com route interception.** Tests never hit real GitHub: a
  `page.route()` interception serves `e2e-browser/fixtures/pr-page.html` at
  `https://github.com/foo/bar/pull/1`, so the manifest's
  `*://github.com/*/pull/*` content-script match fires for real. The daemon
  is spawned with `extension_origin = "https://github.com"` so CORS matches,
  and Private-Network-Access features are disabled via flags (an unpacked
  `--load-extension` doesn't get the PNA exemption installed extensions get).
- **Serial by design.** `workers: 1` — extension loading is heavy and the
  spawned daemons hold real ports.

Debugging: `pnpm test:browser:headed` or `PWDEBUG=1`/`HEADED=1` to watch the
browser; traces and screenshots are retained on failure
(`test-results/`).

## CI matrix — what blocks merge

`.github/workflows/ci.yml`, on every PR and push to `main`:

| Job | OS | Steps |
|---|---|---|
| `rust` | ubuntu, macos, windows | `cargo fmt --check` (ubuntu only) · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace --all-features` with `RUSTFLAGS=-D warnings` and `LIBRE_CR_E2E_REQUIRED=1` |
| `extension` | ubuntu | `pnpm install --frozen-lockfile` · `pnpm typecheck` · `pnpm test` |
| `browser` | ubuntu | Playwright Chromium install · pre-`cargo build` of both daemons (cleaner failure surface) · `pnpm test:browser` with `LIBRE_CR_E2E_REQUIRED=1` |

All jobs block merge. Locally, the pre-PR checklist in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md) reproduces it.

## When to add to which layer

- Parsing/logic bug → unit test next to the code.
- Route/frame behavior, auth, session lifecycle → review-crate integration
  test (`tests/http_api.rs` et al.) — fast, in-process, no binaries.
- Anything about the *wire contract between processes* (envelope shapes,
  spawn/supervision, stdio framing) → `libre-cr-e2e`.
- Extension logic/DOM → vitest unit; extension-against-daemon wiring → node
  E2E; anything needing a real browser (content-script injection, Shadow
  DOM, pairing UX) → Playwright.
- Resist adding browser tests for things a vitest can catch — the browser
  suite costs ~minutes; the unit suite costs seconds.
