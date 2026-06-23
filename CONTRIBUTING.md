# Contributing

Thanks for your interest. Start with the developer docs in
[`docs/development/`](docs/development/README.md) — they cover the
architecture as built, project conventions, the full test taxonomy, and
step-by-step recipes for the common extension points (new tools, verbs,
providers, routes).

## The short version

- **Specs are the source of truth.** Read the relevant files under
  [`specs/`](specs/) before changing behavior; code matches the spec, not
  the other way around. If the spec is wrong, fix the spec first (a separate
  PR is fine). Check [`specs/plan.md`](specs/plan.md) for the current phase
  and flag out-of-phase work explicitly.
- **Wire-format changes** (HTTP routes, WS frames, MCP tool signatures,
  error codes) require — in the same PR — the spec edit, the typed struct in
  `crates/libre-cr-common/`, the mirror in
  `extension/utils/daemon/frames.ts`, and a `PROTOCOL_VERSION` bump if
  breaking. Details:
  [docs/development/conventions.md](docs/development/conventions.md).
- Keep commits focused (one logical change each); PR descriptions name the
  spec section they implement or revise.

## Local checks (what CI enforces)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
LIBRE_CR_E2E_REQUIRED=1 cargo test --workspace
```

For the extension:

```sh
cd extension
pnpm install
pnpm typecheck
pnpm test
pnpm test:browser   # Playwright; needs `pnpm exec playwright install chromium` once
```

CI runs the Rust suite on Linux, macOS, and Windows; all jobs block merge.
See [docs/development/testing.md](docs/development/testing.md) for the full
suite map and how to run any single test.

## Reporting issues

GitHub Issues. Selector-mismatch reports for the GitHub adapter (the
extension's scrape/selection breaking on a GitHub UI change) are especially
welcome — include the PR page variant (classic / new diff view) if you can.

## License

By contributing you agree your contributions are licensed under the MIT
license, same as the rest of the project.
