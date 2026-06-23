# Development Setup

## Toolchain

| Tool | Version | Pinned by |
|---|---|---|
| Rust | stable channel (MSRV 1.82) | `rust-toolchain.toml` (also pulls `rustfmt` + `clippy`) |
| Node | 20+ | `.github/workflows/ci.yml` uses Node 20 |
| pnpm | 9.x | `extension/package.json` `"packageManager": "pnpm@9.12.0"` |
| git | any modern | the code daemon shells out to `git` for `fetch`/`worktree add` |

```sh
# Rust: rustup picks up rust-toolchain.toml automatically
rustup show            # should report the stable toolchain + rustfmt, clippy

# Node/pnpm via corepack (ships with Node)
corepack enable
corepack prepare pnpm@9 --activate
```

## First build

```sh
# Rust workspace — builds all five crates and three binaries
cargo build --workspace

# Extension
cd extension
pnpm install
pnpm build          # produces .output/chrome-mv3/
```

`cargo test --workspace` on a cold checkout also works — the E2E suites in
`crates/libre-cr-e2e/` lazily `cargo build` the binaries they spawn (and
*skip* if the build fails, unless `LIBRE_CR_E2E_REQUIRED=1`; see
[testing.md](testing.md)).

## Running the daemons directly

Each binary is independently runnable, which is the normal dev loop —
you rarely need the wrapper.

```sh
# Review daemon: HTTP/WS server (Serve is the default subcommand)
cargo run -p libre-cr-review -- serve
cargo run -p libre-cr-review -- serve --config /path/to/review.toml

# Code daemon: MCP on stdio is the default; the CLI surface is handy alone
cargo run -p libre-cr-code -- tools      # dump every MCP tool + schema
cargo run -p libre-cr-code -- doctor     # env sanity check
cargo run -p libre-cr-code -- scan --roots ~/Dev   # register local repos
cargo run -p libre-cr-code -- mcp-stdio  # what the review daemon spawns

# Wrapper CLI (binary is named `libre-cr`; crate is libre-cr-cli)
cargo run -p libre-cr-cli -- status
cargo run -p libre-cr-cli -- start       # supervises libre-cr-review from PATH
```

Logging is `tracing` + `RUST_LOG` (defaults to `info`, written to stderr):

```sh
RUST_LOG=debug,libre_cr_review=trace cargo run -p libre-cr-review -- serve
```

On startup `serve` writes three files the extension (and tests) discover it
by: `~/.config/libre-cr/endpoint` (URL — the port is OS-assigned because the
default `server.port` is `0`), `~/.config/libre-cr/token` (bearer token), and
`~/.config/libre-cr/install_key`. State lands in
`~/.local/share/libre-cr-review/state.db` and
`~/.local/share/libre-cr-code/` (see the state table in
[architecture.md](architecture.md)).

**Gotcha — silent mock fallback.** If `serve` cannot spawn the code daemon
binary, it logs a warning and falls back to `MockCodeDaemonClient`
(`crates/libre-cr-review/src/cli.rs`). You get fake code-intel with no hard
error. The review daemon resolves the binary from the `code_daemon.binary`
config value (default: `libre-cr-code` on `PATH`), so for development either
add `target/debug` to `PATH` or point the config at it:

```toml
# review.toml
[code_daemon]
mode = "spawn"
binary = "/path/to/libre-cr/target/debug/libre-cr-code"
```

The same applies to `libre-cr start`: the wrapper spawns `libre-cr-review`
from `PATH` (`crates/libre-cr-cli/src/supervisor.rs`).

## Recommended local config

Config lives at `~/.config/libre-cr/review.toml`
(`crates/libre-cr-review/src/config.rs` documents every section and default).
A useful dev config:

```toml
[provider]
kind = "mock"          # deterministic; no API key needed (default)
# kind = "anthropic"   # or "openai_compat" — set the key via the config UI
#                      # or POST /v1/config (keys are stored encrypted as
#                      # api_key_enc, never plaintext)
model = "mock-model"

[code_daemon]
binary = "/path/to/libre-cr/target/debug/libre-cr-code"

[mock]
# code_intel = true    # sessions get a fake worktree path instantly —
#                      # useful for pure-extension work with no repos set up
```

The **mock provider** is the default and what every test suite uses. It can
replay a scripted event stream (`[[mock.provider_script]]` /
`MockConfig::provider_script` in `config.rs`) — see
`crates/libre-cr-e2e/tests/http_consumer.rs` for examples of scripting
text deltas, tool calls, and presentation calls.

To exercise a real provider, run the config UI (`libre-cr config`, or POST to
`/v1/config`) — provider changes hot-swap without a restart
(`ProviderHandle` in `crates/libre-cr-review/src/server/state.rs`).

## Extension dev loop

```sh
cd extension
pnpm dev            # WXT watch mode: rebuilds + launches Chromium with the
                    # extension loaded and hot-reloads on save
pnpm dev:firefox    # same against Firefox
```

With `pnpm dev` running and a local `cargo run -p libre-cr-review -- serve`:

1. Open a GitHub PR page in the WXT-launched browser.
2. Open the extension options page and pair: run
   `cargo run -p libre-cr-review -- pair` to print a pairing code, paste it
   into the options page (it POSTs to `/v1/pair` and stores the token).
3. The content script (`extension/entrypoints/content/index.ts`) attaches the
   panel; selections + questions round-trip over WS.

Fast checks while iterating:

```sh
pnpm typecheck      # tsc --noEmit (strict mode)
pnpm test           # vitest unit suite (~6 s)
```

## Editor notes

- Rust: clippy with `-D warnings` is the CI bar — configure your editor to
  match (`cargo clippy --workspace --all-targets -- -D warnings`).
- TS: the project tsconfig extends WXT's generated `.wxt/tsconfig.json`;
  run `pnpm install` (which triggers `wxt prepare`) before opening the
  extension dir or the editor will report missing types.
