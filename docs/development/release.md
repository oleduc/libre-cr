# Releases and Versioning

Honest current state: **there is no end-to-end release pipeline yet.** What
exists is a working CI gate, a cross-compile stub, and the versioning
machinery the pipeline will eventually enforce. Target design:
[`specs/08-distribution.md`](../../specs/08-distribution.md).

## What exists today

- **CI** (`.github/workflows/ci.yml`) — the merge gate. Builds and tests the
  workspace on Linux/macOS/Windows plus the extension and browser-E2E jobs.
  See [testing.md](testing.md).
- **Release stub** (`.github/workflows/release.yml`) — triggers on `v*` tags
  or manual dispatch (`dry_run` defaults to true). Cross-compiles release
  binaries (`libre-cr`, `libre-cr-review`, `libre-cr-code`) for five targets:

  | Target | Runner |
  |---|---|
  | `x86_64-unknown-linux-gnu` | ubuntu-latest |
  | `aarch64-unknown-linux-gnu` | ubuntu-latest + `gcc-aarch64-linux-gnu` cross-linker |
  | `x86_64-apple-darwin` | macos-13 |
  | `aarch64-apple-darwin` | macos-latest |
  | `x86_64-pc-windows-msvc` | windows-latest |

  It stages and uploads artifacts — and stops there. No GitHub Release is
  created, nothing is signed, nothing is published.
- Release profile is tuned in the workspace `Cargo.toml` (`lto = "thin"`,
  `codegen-units = 1`, stripped symbols).
- `libre-cr update` is an explicit stub (`crates/libre-cr-cli/src/update.rs`)
  that tells the user to update via their install channel.

## What Phase 7.5 still needs

Per spec 08 § Build And Release Pipeline, in rough dependency order:

1. **Signing**: macOS codesign + notarization; Windows Authenticode; Linux
   detached GPG signatures.
2. **Publishing**: create the GitHub Release from the staged artifacts;
   zipped extension artifacts for Chrome Web Store / Firefox Add-ons
   (`pnpm zip` / `pnpm zip:firefox` already produce them locally).
3. **Install channels**: Homebrew formula + `brew services` recipe; Scoop
   bucket; `install.sh` for Linux (plus systemd user unit); auto-bump of
   formulas from a release manifest.
4. **Update manifest**: signed JSON at `https://api.libre-cr.dev/latest`
   (version + per-arch URLs + checksums), hardcoded public key in the
   wrapper, and the real `libre-cr update` flow (download → verify → stop →
   swap → restart).

## Versioning policy

Two version numbers with different jobs:

- **Package version** — single workspace version in the root `Cargo.toml`
  (`[workspace.package] version`, currently `0.1.0`); all crates inherit it,
  and the daemons report it (`env!("CARGO_PKG_VERSION")` → `GET /v1/health`
  `version`, `libre-cr-* version` subcommands). The extension has its own
  `extension/package.json` version (also `0.1.0`); keep them moving together
  at release time. Release tags are `v<version>`.
- **`PROTOCOL_VERSION`** — the wire-protocol version
  (`crates/libre-cr-common/src/version.rs`, currently `1`), mirrored as a
  constant in `extension/utils/daemon/frames.ts`. The daemon reports it in
  `GET /v1/health`; the extension soft-warns on mismatch
  (`extension/utils/daemon/protocol.ts`).

**Bump rules for `PROTOCOL_VERSION`** (see also
[conventions.md](conventions.md) § Wire-format change protocol):

- Bump on any *breaking* change to the HTTP/WS surface or MCP tool
  signatures: removed/renamed fields or frames, changed semantics, new
  *required* request fields.
- Don't bump for additive, serde-defaultable changes (new optional fields,
  new frame types — the extension drops unknown frames, serde ignores
  unknown fields). That tolerance is the mechanism behind the wire-compat
  promise, so additions must always be optional-with-default on both sides.
- Bump both constants (Rust + TS) in the same PR as the spec edit.

**The wire-compat promise** (spec 08): minor package versions stay
wire-compatible; we publish a compatibility matrix and try hard not to break
extension↔daemon pairing between minor versions.

## Compatibility matrix (from spec 08 § Compatibility Pairing)

| Extension | Daemon | Behavior |
|---|---|---|
| Same | Same | Normal. |
| Newer | Older | Extension feature-detects via `GET /v1/health` (`version`, `protocol_version`); degrades or prompts to update the daemon. |
| Older | Newer | Daemon keeps old routes working (minor versions back-compatible); normal. |
| Major / protocol mismatch | — | Extension refuses to pair and shows an update prompt. Today's implementation is softer than the spec: a console warning + `ui.protocol_mismatch` diagnostics key, never a hard refusal. |

> **Spec vs implementation:** the spec's "extension refuses to pair on major
> mismatch" is not implemented — `recordProtocolCheck`
> (`extension/utils/daemon/protocol.ts`) deliberately treats every mismatch
> as a soft warning while the protocol is at v1 and there is nothing to
> refuse. When v2 ever exists, the hard-refusal path needs to be written.

## Cutting a release (current manual procedure)

1. Bump `[workspace.package] version` and `extension/package.json`; update
   `Cargo.lock` (`cargo check`).
2. Verify the full gate locally: `cargo fmt --all -- --check && cargo clippy
   --workspace --all-targets -- -D warnings && LIBRE_CR_E2E_REQUIRED=1 cargo
   test --workspace`, then `pnpm typecheck && pnpm test && pnpm test:browser`
   in `extension/`.
3. Tag `v<version>` and push — `release.yml` builds and uploads the
   per-target artifacts.
4. Everything after that (release notes, store uploads) is manual until
   Phase 7.5.
