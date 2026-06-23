# Distribution

## What Ships

| Artifact | Form | Where |
|---|---|---|
| `libre-cr-code` | Single static binary, per OS+arch | GitHub Releases + Homebrew + Scoop |
| `libre-cr-review` | Single static binary, per OS+arch | GitHub Releases + Homebrew + Scoop |
| `libre-cr-ext` | Browser extension package | Chrome Web Store + Firefox Add-ons |
| Wrapper CLI: `libre-cr` | Tiny binary that supervises both daemons | Same channels |

Three artifacts the user installs: the extension (browser-side) and the wrapper CLI (which supervises both daemons). They do not install the daemons directly in v2.

## Why A Wrapper CLI

`libre-cr` is a thin supervisor that:

- Starts/stops both daemons.
- Pairs the extension (generates pairing code).
- Reads logs.
- Reports health.
- Handles updates.

This lets the user think about *one* thing ("the libre-cr service") rather than *two daemons* and their inter-process plumbing.

The wrapper is ~500 lines of Rust that calls into the same crate the daemons are built from, plus a process supervisor.

## Install Paths

### macOS — Homebrew (primary)

```
brew install libre-cr
```

Installs `libre-cr`, `libre-cr-review`, `libre-cr-code` to `/opt/homebrew/bin` (or `/usr/local/bin` on Intel). Sets up no services by default; user runs `libre-cr start` to launch.

A `brew services start libre-cr` recipe is also provided for users who want the service to autostart at login.

### Linux

```
# Debian/Ubuntu
curl -fsSL https://libre-cr.dev/install.sh | sh
# or distro package once we're shipping deb/rpm
```

Installs to `/usr/local/bin` or `$HOME/.local/bin`. systemd unit file generated for users who want autostart:

```
systemctl --user enable --now libre-cr
```

### Windows — Scoop

```
scoop bucket add libre-cr https://github.com/libre-cr/scoop
scoop install libre-cr
```

Installs to `%USERPROFILE%\scoop\apps\libre-cr\current`. For autostart, a Task Scheduler entry is offered during `libre-cr start --autostart`.

### Manual

Download the appropriate archive from GitHub Releases, unpack, put the three binaries on PATH. The wrapper is self-contained.

### Browser extension

Standard Web Store / Add-ons install. The extension is independent of the binaries; installing one without the other shows a friendly "pair with daemon" or "install daemon" message.

## First-Run Flow

```
$ libre-cr start
libre-cr v0.1.0
  ✓ libre-cr-review started (PID 4012) on http://127.0.0.1:7841
  ✓ libre-cr-code  started (PID 4013) via stdio
  ✓ token written to ~/.config/libre-cr/token (mode 0600)
  ✓ endpoint written to ~/.config/libre-cr/endpoint

To pair the browser extension:
  1. Open a GitHub PR in your browser.
  2. Click the libre-cr icon or open the extension's options page.
  3. Click "Pair with daemon".
  4. Run `libre-cr pair` and paste the code that appears.

Logs:    ~/.local/state/libre-cr/log/
Config:  ~/.config/libre-cr/
Status:  libre-cr status
Stop:    libre-cr stop
```

The pairing dance described in `04-review-daemon.md` and `05-browser-extension.md` is what `libre-cr pair` runs.

## Wrapper CLI Surface

```
libre-cr start [--autostart]      Start both daemons (idempotent)
libre-cr stop                     Stop both daemons gracefully
libre-cr restart
libre-cr status                   Show health, version, ports, PIDs
libre-cr logs [-f]                Tail both daemons' logs
libre-cr pair                     Run the extension pairing flow
libre-cr config                   Open the review daemon's config UI
libre-cr doctor                   Diagnose: ports, file perms, code-daemon health
libre-cr update                   Check for updates; apply if user confirms
libre-cr version
libre-cr uninstall                Stop daemons, prompt before removing data
```

## Supervision Model

The wrapper supervises both daemons. Failure handling:

- `libre-cr-review` exits unexpectedly → wrapper restarts up to 5 times in 60 s, then surfaces an error and stops.
- `libre-cr-code` exits unexpectedly → `libre-cr-review` notices (its MCP child died) and restarts it via the same supervision logic from its own side. The wrapper just keeps `libre-cr-review` alive.

Two layers of supervision because the user-facing process (`libre-cr-review`) cares about `libre-cr-code` for every operation; the wrapper cares about the user-facing process. Both are well-tested patterns.

Logs go to `~/.local/state/libre-cr/log/`:
- `libre-cr-review.log` (rolling, daily, 14 days retained)
- `libre-cr-code.log` (same)
- `supervisor.log` (start/stop events, restart counts)

## Configuration Layout

```
~/.config/libre-cr/
├── token                       # Daemon bearer token (mode 0600, generated)
├── endpoint                    # http://127.0.0.1:<port> for the extension
├── review.toml                 # Review daemon config (provider, model, key)
└── code.toml                   # Code daemon config (storage paths, eviction)

~/.local/share/libre-cr/        # Shared parent for daemon data
~/.local/share/libre-cr-review/
├── state.db                    # SQLite: sessions, turns, notes, traces
~/.local/share/libre-cr-code/
├── state.db                    # SQLite: repos, worktrees
├── repos/                      # Auto-cloned repos
└── worktrees/                  # PR worktrees (LRU-evicted)

~/.local/state/libre-cr/log/    # Logs
```

These follow the XDG Base Directory spec on Linux/macOS and equivalent locations on Windows (`%APPDATA%`, `%LOCALAPPDATA%`).

## Updates

Phase B is manual:

```
libre-cr update
  Current: 0.1.3
  Latest:  0.1.5

  Changelog: https://libre-cr.dev/releases/0.1.5

  Apply? [y/N]
```

Implementation:
- Hit `https://api.libre-cr.dev/latest` (signed JSON manifest with version + per-arch URLs + checksums).
- Verify signature against a hardcoded public key.
- Download new binaries to a temp dir, verify checksums.
- Stop daemons, swap binaries, restart.

Browser extension updates flow through the browser's normal extension update mechanism. The extension's stored version is checked against the daemon's reported version on every session init — mismatch shows a soft warning if the gap matters.

Future: auto-update opt-in flag in config.

## Compatibility Pairing

| Extension version | Daemon version | Behavior |
|---|---|---|
| Same | Same | Normal |
| Newer extension, older daemon | Older daemon doesn't recognize a new HTTP route | Extension feature-detects via `GET /v1/health`'s version; degrades or prompts to update |
| Older extension, newer daemon | Daemon supports old routes (semver: minor versions stay back-compatible) | Normal |
| Major mismatch | Extension refuses to pair, shows update prompt | |

We publish a compatibility matrix in the docs and try not to break wire compatibility between minor versions.

## Telemetry And Privacy

Default: zero outbound telemetry. The daemons phone home only for the version check during `libre-cr update`.

Opt-in (future): anonymized usage stats. Specifically:
- Verbs run, count per session, no content.
- LLM provider used (name only, no key, no usage).
- Tool call counts, latencies.
- Error categories.

Never sent: code content, PR content, conversation content, notes, file paths.

Phase B ships with the opt-in off and no telemetry server. Adding telemetry is a deliberate, separately-shipped change.

## Uninstall

```
libre-cr uninstall
  This will:
    • Stop the libre-cr daemons.
    • Remove binaries from /opt/homebrew/bin (etc.).
    • Remove config from ~/.config/libre-cr.

  Keep data (~/.local/share/libre-cr-*)? [Y/n]
  Keep logs (~/.local/state/libre-cr)?    [Y/n]
```

Browser extension is uninstalled through the browser as normal.

## Build And Release Pipeline

CI builds:

- `libre-cr-code`, `libre-cr-review`, `libre-cr` for: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- Browser extension: zipped artifacts for Chrome and Firefox.

Signed artifacts:

- macOS: codesigned + notarized.
- Windows: Authenticode signed.
- Linux: detached GPG signature.

Release on a tag push. Homebrew / Scoop formulas auto-bumped from the release manifest.

## Required Host Software

| Component | Required | Why |
|---|---|---|
| `git` (CLI) | Required | Code daemon uses git for `fetch` and `worktree add` |
| Specific LSPs (`rust-analyzer`, `gopls`, …) | Phase C, optional | Code daemon detects and uses if present |
| A modern browser (Chrome ≥ 120, Firefox ≥ 121) | Required for extension | Manifest V3 |
| LLM API key (Anthropic / OpenAI / compatible) | Required at runtime | Daemon makes the calls |

We do not bundle any of these. `libre-cr doctor` checks for git and warns clearly if it's missing.

## Multi-Machine Considerations

A user with multiple machines (laptop + workstation) installs the daemons on each. Each has its own conversation history and pairing. There is no sync. Two reasons:

1. Sync would require either a server we operate (rejected) or a peer-to-peer protocol (out of scope for v2).
2. The conversation history is tied to a local checkout that's also not synced. Without the same code at the same commit, replaying a conversation is incoherent.

Users who want cross-machine continuity can use the (future) GitHub-posted review as the durable artifact, which is exactly what GitHub is good at.

## Security Considerations (Distribution-Level)

- Signed binaries; checksum-verified downloads.
- Token written 0600. Endpoint file too.
- Token is regenerated on `libre-cr restart --rotate-token` (also re-pairs the extension).
- Daemon binds 127.0.0.1 only — never `0.0.0.0`. Configurable for advanced uses, but documented as "you are now responsible for network ACLs."
- Daemon refuses to start with a config file with mode wider than 0644.
- `libre-cr doctor` checks file permissions and reports issues.

## What Distribution Doesn't Do (Phase B)

- No package for nix, arch user repository, or distro-specific channels beyond the listed ones. Community contributions welcome.
- No mobile or iPad install. There is no value proposition there.
- No portable install (USB stick). The XDG locations are assumed.
- No corporate proxy / firewall auto-configuration. If the LLM provider isn't reachable, that's the user's network. We surface the error clearly.
