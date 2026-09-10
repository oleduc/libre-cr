# Distribution

## What Ships

> **Status: no packaging artifact exists in this repository** — no formula,
> no `install.sh`, no service unit, no Scoop manifest. `release.yml` labels
> itself a Phase 0 stub and defers codesigning, notarization, Authenticode,
> GPG and formula bumps to Phase 7. Everything in this section and in
> § Install Paths, § Updates and § Build And Release Pipeline is the intended
> distribution story; today the binaries are built with `cargo build
> --release` and run from the checkout (a `direnv` `PATH_add target/release`
> is the working setup). Marked future rather than dropped: `plan.md` Phase 7
> carries it.

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

The wrapper is ~2,000 lines of Rust. It does **not** link the daemon crates — it depends on `libre-cr-common` only and spawns `libre-cr-review` by name from `PATH`, which is why a stale copy earlier on `PATH` silently shadows a fresh build.

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
  ✓ libre-cr-review started (PID 4012) on http://127.0.0.1:54321
  ✓ libre-cr-code  started (PID 4013) via stdio
  ✓ token written to ~/.config/libre-cr/token (mode 0600)
  ✓ endpoint written to ~/.config/libre-cr/endpoint

To pair the browser extension:
  1. Run `libre-cr pair` — it mints a one-time code through the running daemon.
  2. Open the extension's options page.
  3. Click "Pair with daemon" and enter that code.

Logs:    ~/.local/state/libre-cr/log/
Config:  ~/.config/libre-cr/
Status:  libre-cr status
Stop:    libre-cr stop
```

The pairing dance described in `04-review-daemon.md` and `05-browser-extension.md` is what `libre-cr pair` runs.

## Wrapper CLI Surface

The port shown is illustrative: the default is an **ephemeral** port (`port = 0`), so it differs per run unless `[server] port` is pinned. The endpoint file is the authority, and `start` removes any stale one before spawning so the banner cannot report a dead port.

```
libre-cr start [--autostart]      Start both daemons (idempotent); --autostart only prints a notice today
libre-cr stop                     Stop both daemons gracefully
libre-cr restart
libre-cr status                   Show health, version, ports, PIDs
libre-cr logs [-f]                Tail both daemons' logs
libre-cr pair                     Issue a pairing code through the running daemon (POST /v1/pair/issue)
libre-cr config                   Open the review daemon's config UI (<endpoint>/config-ui?token=…)
libre-cr doctor                   Diagnose: git, binaries on PATH, file perms, endpoint format
libre-cr update                   Check for updates; apply if user confirms
libre-cr version
libre-cr uninstall                Stop daemons, prompt before removing data
```

## Supervision Model

The wrapper supervises both daemons. Failure handling:

- `libre-cr-review` exits unexpectedly → wrapper restarts up to 5 times in 60 s, then surfaces an error and stops.
- `libre-cr-code` exits unexpectedly → `libre-cr-review` notices (its MCP child died) and restarts it via the same supervision logic from its own side. The wrapper just keeps `libre-cr-review` alive.

Two layers of supervision because the user-facing process (`libre-cr-review`) cares about `libre-cr-code` for every operation; the wrapper cares about the user-facing process. Both are well-tested patterns.

`libre-cr start` runs the supervisor **in the foreground** — it does not daemonize itself. After printing the first-run summary it holds the terminal, supervising the review daemon and honoring `SIGTERM`/`SIGINT` for graceful shutdown. Turning it into a background service is the platform service manager's job: `brew services` (macOS), `systemd --user` (Linux), or Task Scheduler (Windows). Double-forking ourselves is fragile across platforms and intentionally avoided.

**Two pid files, and only one of them is the install.** The supervisor records
the review daemon's pid in `run/review.pid` — that pid changes on every
restart. It records *itself* in `run/supervisor.pid`, and that is what "is
libre-cr running?" means:

- `libre-cr stop` signals the **supervisor**. Its own `SIGTERM` handler stops
  the daemon gracefully and leaves the restart loop. Signalling the daemon
  directly only makes the supervisor spawn a replacement ~250 ms later, on a
  fresh port, after which `start` reports "already running".
- `stop` then reaps a daemon that outlived its supervisor, and `start` clears
  such an orphan before binding. A force-killed wrapper cannot stop its child,
  so the daemon can survive holding the port — which `start` used to read as a
  healthy install and refuse to replace.
- `status` reports the two separately and flags `running unsupervised` when it
  sees an orphan.

Escalation is uniform: `SIGTERM`, wait, then `SIGKILL`.

Logs go to `~/.local/state/libre-cr/log/`:
- `libre-cr-review.log` — the supervisor's append-only capture of the daemon's stderr
- `libre-cr-code.log` — same, for the code daemon
- `supervisor.log` — start/stop events, restart counts

**No rotation and no retention window.** The daemons log to stderr only; the
supervisor appends that stream to these files and never truncates them, so
they grow unbounded. Not planned: for a local single-user tool, `libre-cr logs`
plus manual deletion is the accepted answer. (`[logging] file` in `code.toml`
is accepted and ignored.)

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

```text
~/.local/state/libre-cr/run/
├── supervisor.pid              # the `libre-cr start` process — the install
└── review.pid                  # the supervised child (changes per restart)
```

`review.toml` also names an `install_key` file (`~/.config/libre-cr/install_key`), which the daemon creates and needs in order to decrypt the stored provider key.

These follow the XDG Base Directory spec on Linux and macOS. **Windows is not special-cased today:** the path helpers are `$XDG_*`-or-`$HOME` on every platform, so a Windows install lands in `%USERPROFILE%\.config\libre-cr` rather than `%APPDATA%`. Intended, not built.

**On macOS the config path is `$XDG_CONFIG_HOME` → `~/.config`, deliberately
not `dirs::config_dir()`.** That helper returns `~/Library/Application Support`
on macOS, so the daemons silently ignored the `review.toml`/`code.toml` that
the wrapper, the docs, and their own token and endpoint files all use — they
served defaults instead, and every test missed it because the harness passes
`--config` explicitly. A one-time migration copies a file stranded at the
Application Support location, and falls back to loading it in place if the copy
fails rather than defaulting over a readable config.

Config sections parse partially: a `[provider]` block carrying only `kind` fills
the rest from defaults instead of failing to parse. A hand-written minimal
config is a supported starting point, and the new `[limits]` caps
(`10-grounding-and-context.md`) reach existing installs this way without an
edit.

## Updates

> **Status: not built.** `libre-cr update` prints "auto-update is not
> implemented yet" and exits — there is no version check, no signature
> verification and no binary swap, so nothing phones home either. The design
> below is Phase 7.5 in `plan.md`.

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
    • Remove config from ~/.config/libre-cr.
    • Remove ~/.local/share/libre-cr and ~/.local/state/libre-cr.
```

Two caveats, both current behaviour rather than intent. It offers one
all-or-nothing confirmation, not separate keep-data / keep-logs prompts. And
`~/.local/share/libre-cr` is *not* the `libre-cr-*` glob: the daemons' real
databases live in `~/.local/share/libre-cr-review/` and
`~/.local/share/libre-cr-code/`, which **survive uninstall**. It also never
removes binaries — there is no install step that placed them.

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

The LLM key can be supplied two ways, in priority order: (1) saved through the config UI (`/config-ui`, stored encrypted in `review.toml`); or (2) the ambient `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` environment variable, used automatically when no key is saved. The config UI reports which env vars it can detect.

## Multi-Machine Considerations

A user with multiple machines (laptop + workstation) installs the daemons on each. Each has its own conversation history and pairing. There is no sync. Two reasons:

1. Sync would require either a server we operate (rejected) or a peer-to-peer protocol (out of scope for v2).
2. The conversation history is tied to a local checkout that's also not synced. Without the same code at the same commit, replaying a conversation is incoherent.

Users who want cross-machine continuity can use the (future) GitHub-posted review as the durable artifact, which is exactly what GitHub is good at.

## Security Considerations (Distribution-Level)

- Signed binaries; checksum-verified downloads.
- Token written 0600. Endpoint file too.
- Token rotation is *intended* via a `libre-cr restart --rotate-token` flag (which would also force a re-pair). **Not built:** `restart` takes no flags and nothing in the tree rotates a token; deleting the token file and restarting is the manual equivalent.
- Daemon binds 127.0.0.1 only — never `0.0.0.0`. Configurable for advanced uses, but documented as "you are now responsible for network ACLs."
- The daemon **should** refuse to start with a config file whose mode is wider than 0644. **Not enforced:** neither daemon inspects config permissions. Future work; `libre-cr doctor` does check and report the token and endpoint file modes today.
- `libre-cr doctor` checks file permissions and reports issues.

## What Distribution Doesn't Do (Phase B)

- No package for nix, arch user repository, or distro-specific channels beyond the listed ones. Community contributions welcome.
- No mobile or iPad install. There is no value proposition there.
- No portable install (USB stick). The XDG locations are assumed.
- No corporate proxy / firewall auto-configuration. If the LLM provider isn't reachable, that's the user's network. We surface the error clearly.
