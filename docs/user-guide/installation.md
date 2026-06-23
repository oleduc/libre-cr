# Installation

Libre CR has two halves you install: the **binaries** (a wrapper CLI plus two daemons) and the **browser extension**. Today both are built from source. Homebrew, Scoop, and an install script are planned but not yet available, and the extension is not yet published to the Chrome Web Store or Firefox Add-ons.

## Prerequisites

| Requirement | Why | Check |
|---|---|---|
| `git` ≥ 2.0 on PATH | The code daemon shells out to git for `fetch` and `worktree add` | `git --version` |
| Rust toolchain (1.82+) | Build the binaries from source | `cargo --version` |
| Node + pnpm | Build the browser extension | `pnpm --version` |
| Chrome ≥ 120 **or** Firefox ≥ 121 | Manifest V3 extension | — |
| An LLM API key (Anthropic, OpenAI, or any OpenAI-compatible endpoint incl. Ollama) | The review daemon makes the LLM calls | — |

No API key yet? You can still install and smoke-test everything with the built-in [mock provider](configuration.md#mock-provider-mode) — it's the default out of the box.

## 1. Build the binaries

```sh
git clone https://github.com/libre-cr/libre-cr
cd libre-cr
cargo build --release
```

This produces three binaries in `target/release/`:

| Binary | Role |
|---|---|
| `libre-cr` | Wrapper CLI: starts/stops/supervises the daemons, pairing, logs, diagnostics |
| `libre-cr-review` | Review daemon: agent loop, conversation storage, LLM provider client |
| `libre-cr-code` | Code daemon: repo-aware code intelligence (MCP server), spawned by the review daemon |

### Put them on PATH

All three must be findable on `PATH` — the review daemon spawns `libre-cr-code` by name. Either:

```sh
# Option A: extend PATH (add to your shell profile to persist)
export PATH="$HOME/path/to/libre-cr/target/release:$PATH"

# Option B: copy them somewhere already on PATH
cp target/release/{libre-cr,libre-cr-review,libre-cr-code} ~/.local/bin/
```

### Planned install paths (not yet available)

- `brew install libre-cr` (macOS)
- `curl -fsSL https://libre-cr.dev/install.sh | sh` (Linux)
- `scoop install libre-cr` (Windows)
- Prebuilt signed binaries on GitHub Releases

## 2. Start the daemons

```sh
libre-cr start
```

Notes on what you'll see:

- The supervisor runs **in the foreground** — keep that terminal open (or use your own service manager; OS-level autostart via `--autostart` is a stub for now and just prints guidance).
- Within a few seconds it prints the daemon's endpoint (e.g. `http://127.0.0.1:54321` — the port is chosen by the OS each start) and the path to the bearer token file.
- Config lands in `~/.config/libre-cr/`, logs in `~/.local/state/libre-cr/log/`.

`libre-cr stop`, `libre-cr restart`, and `libre-cr status` work from any other terminal.

## 3. Build and load the extension

```sh
cd extension
pnpm install
pnpm build              # Chrome → .output/chrome-mv3/
pnpm build:firefox      # Firefox → .output/firefox-mv2/
```

**Chrome:** open `chrome://extensions`, enable *Developer mode* (top-right toggle), click *Load unpacked*, select `extension/.output/chrome-mv3/`.

**Firefox:** open `about:debugging#/runtime/this-firefox`, click *Load Temporary Add-on…*, select the `manifest.json` inside `extension/.output/firefox-mv2/`. (Firefox temporary add-ons are removed on browser restart — reload after restarting.)

Until the extension ships on the Web Store / AMO, this unpacked load is the supported path.

## 4. Pair the extension with the daemon

With the daemon running:

```sh
libre-cr pair
```

This asks the **running** daemon to mint a one-time pairing code and prints it:

```
Pairing code: 483-291
```

Then, in the browser:

1. Open the extension's options page (toolbar icon → *Pair extension*, or right-click the icon → Options).
2. Enter the **endpoint** printed by `libre-cr start` (also stored in `~/.config/libre-cr/endpoint`) — the pre-filled `http://127.0.0.1:8765` is only a placeholder; your actual port differs.
3. Paste the pairing code and click *Pair*.

The code is single-use, expires after ~5 minutes, and is only valid while the daemon is running. On success the daemon hands the extension its bearer token and permanently allowlists the extension's origin for CORS (persisted into `review.toml` automatically — see [Configuration](configuration.md#extension_origin-and-cors)).

The options page also understands a pairing deep-link (`options.html#pair?endpoint=…&code=…&auto=1`) that pre-fills or auto-completes the form; manual paste always works.

## 5. Verify

```sh
libre-cr doctor
```

A healthy install looks like:

```
  ✓ git on PATH            found git version 2.44.0
  ✓ libre-cr-review binary found at /Users/you/.local/bin/libre-cr-review
  ✓ libre-cr-code binary   found at /Users/you/.local/bin/libre-cr-code
  ✓ token file perms       ~/.config/libre-cr/token is 0600
  ✓ endpoint file perms    ~/.config/libre-cr/endpoint is 0600
  ✓ daemon endpoint        http://127.0.0.1:54321 (ping via `libre-cr status`)
  · disk space (worktrees) not measured in this build
```

`libre-cr status` additionally pings the daemon's `/v1/health`. Then open any GitHub PR — the floating Libre CR button should appear and show as connected.

## Updating

`libre-cr update` is **not implemented yet** (it prints a notice). To update: `git pull`, rebuild (`cargo build --release`, `pnpm build`), `libre-cr restart`, and reload the unpacked extension from `chrome://extensions`.

## Next steps

- Point the daemon at a real LLM provider: [Configuration](configuration.md)
- Learn the review workflow: [Using Libre CR](using.md)
