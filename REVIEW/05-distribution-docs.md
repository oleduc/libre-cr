# Distribution, docs, and ops review — libre-cr v2

## Summary

The wrapper CLI's *shape* matches `specs/08-distribution.md` almost exactly:
every documented subcommand exists, the supervisor with the 5-in-60s restart
policy is implemented and unit-tested against `true`/`false`/`sleep`, PID-file
lifecycle and signal handling are correct on Unix, and XDG paths land where the
spec says. CI runs fmt/clippy/test on three OSes plus extension typecheck;
`release.yml` is a believable stub. The Phase-7 deferrals (Brew, Scoop,
install.sh, signed releases, autostart, auto-update) are explicitly labeled in
code and message strings.

The verdict is **not shippable as-is**: the first-run pairing flow is broken,
`libre-cr config` opens a 404, and `libre-cr-code`'s stderr is discarded
entirely, so users will hit problems they can't diagnose. None of these are
architectural — they're plumbing gaps between Phase 2 and Phase 7. A few hours
of work would close the critical set.

## Critical (block v2)

1. **`libre-cr pair` does not actually pair.** `crates/libre-cr-cli/src/pair.rs`
   simply generates a random 8-char code from a local alphabet and prints it.
   The daemon's `PairingStore` (`crates/libre-cr-review/src/pairing.rs:25`)
   issues 12-hex-char codes from a *separate* in-memory store. The extension
   POSTs the code to `/v1/pair`, which calls `state.pairing.redeem(&code)`
   (`crates/libre-cr-review/src/server/routes.rs:445`). The wrapper's printed
   code is unknown to the daemon, so redemption returns `Unauthorized`. To
   actually pair today the user must run `libre-cr-review pair` (the daemon's
   own subcommand at `crates/libre-cr-review/src/cli.rs:71`) instead of
   `libre-cr pair`. The wrapper either needs to shell out to that subcommand,
   or the daemon needs a `GET /v1/pair/issue` endpoint that the wrapper hits.

2. **`libre-cr config` opens a non-existent URL.**
   `crates/libre-cr-cli/src/commands/config.rs:11` constructs
   `<endpoint>/config-ui` and shells `webbrowser::open`. The review daemon
   serves `/v1/config` (GET/POST) and `/v1/config/validate`
   (`crates/libre-cr-review/src/server/routes.rs:78-79`) — there is no
   `/config-ui` route. The user lands on a 404.

3. **The code daemon's logs are dropped on the floor.**
   `crates/libre-cr-review/src/code_daemon/transport.rs:83` sets
   `.stderr(Stdio::null())` when spawning `libre-cr-code`. The wrapper's
   supervisor only redirects the review daemon's stdout/stderr to
   `libre-cr-review.log`. There is no `libre-cr-code.log` (spec § Supervision
   Model line 121 requires one). When a code-daemon tool call fails, the user
   has nothing to grep. The fix is either (a) pipe code-daemon stderr into a
   dedicated log file in the review daemon, or (b) inherit through to the
   wrapper's log so the supervisor captures it.

4. **`code.toml` does not exist.** `specs/08-distribution.md:127-132` declares
   `~/.config/libre-cr/{review.toml, code.toml}` as the ownership boundary, but
   `crates/libre-cr-code/src/config.rs:116-122` defaults to
   `~/.config/libre-cr-code/config.toml` (a separate XDG namespace). Either the
   spec needs to relax, or the code daemon needs to follow the wrapper's
   namespace. Either way the wrapper has no awareness of `code.toml` and will
   not surface its existence in `doctor`.

## Important

5. **No `libre-cr update` for v2.** `crates/libre-cr-cli/src/update.rs` prints
   a "Phase 7.5" stub. Today's user experience is "update by the channel you
   installed from"; in practice that means `cargo install --git …` since none
   of the channels (Brew/Scoop/install.sh) exist yet. There is no recipe in
   README or CONTRIBUTING for what to actually run.

6. **`libre-cr doctor` is missing two of the four spec-listed checks.**
   Spec line 198 says "git presence, port availability, file permissions,
   code-daemon health". Implemented: git ✓, file perms ✓.
   Missing: **port availability** (no `TcpListener::bind` probe in
   `doctor.rs`) and **code-daemon health** (the binary-on-PATH check is a
   shallow proxy; no `/v1/health` ping — that lives only in `status.rs`).
   Disk-space is explicitly punted with a `Skip`.

7. **Endpoint discovery for the extension is on the user.** Review daemon
   binds to port 0 (ephemeral; `crates/libre-cr-review/src/config.rs:47`) and
   writes the chosen URL to `~/.config/libre-cr/endpoint`. The extension's
   options page (`extension/entrypoints/options/Options.tsx:14`) defaults to
   `http://127.0.0.1:8765` and asks the user to "paste the endpoint". Nothing
   in the wrapper output or docs tells the user to `cat
   ~/.config/libre-cr/endpoint`. The first-run banner in
   `commands/start.rs:60` does print the endpoint *once*, but only on
   successful first start, and only if the user is still looking at the
   terminal.

8. **Windows graceful stop is a no-op.** `crates/libre-cr-cli/src/proc.rs:89`
   and `:106` have `send_term` / `send_kill` as no-ops on Windows ("Without
   windows-sys we can't send a soft stop signal"). The supervisor's shutdown
   path (`supervisor.rs:200-211`) sends SIGTERM, waits 5s, then `start_kill`.
   On Windows, `send_term` does nothing, so the wait is wasted; `start_kill`
   (TerminateProcess) is the real stop, so children always get hard-killed
   with no graceful window. Document this — it means in-flight tool calls
   are lost and SQLite WAL recovery runs on next boot.

9. **`libre-cr-code` logs aren't surfaced by `libre-cr logs`.**
   `commands/logs.rs:8` tails only `review_log_file()` + `supervisor_log_file()`.
   Even once issue 3 is fixed, `paths.rs` has no `code_log_file()`. A
   `libre-cr logs -f` user won't see code-daemon messages.

10. **Log rotation is a TODO.** `logs.rs:1-6` is explicit: daily rotation +
    14-day retention (spec line 122) is unimplemented. `libre-cr-review.log`
    will grow unbounded. Acceptable for v2 if documented; today CONTRIBUTING
    doesn't mention it.

11. **CONTRIBUTING omits the wrapper.** It tells contributors to run
    `cargo fmt/clippy/test` and `pnpm typecheck/test`, but never mentions
    `libre-cr start`, the supervisor, or the integration test that wires a
    fake review daemon (`crates/libre-cr-cli/tests/integration.rs`). A new
    contributor would not know the wrapper's smoke tests need `$HOME` /
    `XDG_*` overrides to be safe.

12. **README does not link to install paths or first-run flow.** It links to
    the specs but doesn't tell the reader *how* to try the thing — there's no
    "build the wrapper, run `libre-cr start`, install the extension from
    source" walkthrough. README also says "Phase 0 (scaffolding)" — stale,
    we're well into Phase 7 territory based on the artifacts.

## Suggestions

- The release workflow (`release.yml`) builds five targets but never tags
  artifacts with version, never produces SHA256 sums, never zips/tars. That's
  fine for Phase 7.5 follow-up but worth a comment in-file.
- `cargo run --quiet -p libre-cr-cli -- --help` output matches the spec
  table 1:1 — good. Worth dropping that table into the wrapper's `--help`
  long-about so the binding stays visible.
- `uninstall` has a nice belt-and-suspenders gate (`LIBRE_CR_UNINSTALL_DRY_RUN`
  env) for tests. Consider also gating destructive `remove_dir_all` calls on a
  sanity check that the targets are under `$HOME` — defense against a
  misconfigured `XDG_*` pointing at `/`.
- `pair.rs` uses `rand::thread_rng().gen_range` from `rand 0.8`. Once
  pairing is wired through the daemon's `PairingStore`, this file collapses to
  a thin HTTP client and the alphabet trick disappears.
- `status` prints fine output but doesn't show *which port* — only the
  endpoint URL. Spec § Wrapper CLI Surface explicitly says "ports". Trivial.
- `.gitignore` ignores `*.log` globally — that will hide
  `~/.local/state/libre-cr/log/*.log` if a user ever runs the wrapper inside
  the repo root, but won't affect normal use.

## First-run walkthrough

What a fresh user does today, given the current code and docs:

1. `git clone … && cargo build --release` — README mentions only
   `cargo build --workspace`, no `--release`, so the binaries the user wants
   on PATH (`target/release/libre-cr*`) require an extra flag they have to
   guess.
2. `cp target/release/{libre-cr,libre-cr-review,libre-cr-code} ~/.local/bin/`.
   Nothing in README or CONTRIBUTING says to do this. README does not mention
   that `libre-cr start` only works if `libre-cr-review` and `libre-cr-code`
   are on `$PATH` (the warning lives in `doctor` output only).
3. `libre-cr doctor` — friendly checklist, but warns "libre-cr-review binary
   not found on PATH" if step 2 was skipped. No remediation pointer.
4. `libre-cr start` — prints a nice banner, then either hangs (no LLM key
   configured → review daemon may still serve, depends on `provider.kind =
   "mock"` default) or runs. The endpoint is printed to stdout. The user
   needs to remember it. There is no recipe for "set my Anthropic key".
   (`/v1/config` exists, but the user has no UI yet — `libre-cr config`
   opens a 404.)
5. User installs the extension by sideloading the unpacked dev build (no Web
   Store listing — spec says Phase 7.5). README does not explain this.
6. User opens extension Options. It asks for an endpoint URL and a code from
   `libre-cr pair`.
7. User runs **`libre-cr pair`** in another terminal → gets an 8-char
   alphabet code. Pastes it. **The daemon returns 401 Unauthorized** because
   the wrapper-generated code was never registered with the daemon.
8. User searches docs, finds none. The actual command they need is
   `libre-cr-review pair`. That's not documented anywhere user-facing.

Friction count: at least four hard stops (PATH, key config, endpoint
discovery, broken pairing), plus the silent-failure mode where code-daemon
errors disappear into `/dev/null`.

## Confirmed good

- Supervisor: `RestartTracker` (`supervisor.rs:86-122`) is a clean sliding
  window with prune-on-record, unit-tested for both budget-exhaustion and
  pruning of old events; the `would_exceed` semantics correctly account for
  the +1 we're about to record.
- PID file handling: tightened to 0600 on Unix
  (`proc.rs:32-38`), stale detection via `kill(pid, 0)` honoring `EPERM`
  (`proc.rs:62-66`), and `start.rs:23-29` clears stale PID before respawn.
- Graceful shutdown: `supervisor.rs:196-215` selects between child exit and a
  cancel oneshot; SIGTERM → 5s → SIGKILL; PID file removed before return.
  Tested with a real `sleep 30` child.
- XDG paths: `paths.rs` honors `$HOME` + `XDG_*` overrides, lazily resolved
  so tests can sandbox. `ensure_dirs` sets 0700 on Unix — defense in depth.
- CI matrix: `ci.yml` runs all three OSes, fmt is Linux-only (correct
  decoupling), clippy + test on every OS. Extension typecheck + vitest are
  gated on `extension/package.json` existing (clean Phase-0 conditional).
- Release workflow: cross-linker for aarch64 Linux is right; matrix covers
  the five targets the spec calls out.
- `doctor` output is structured (`CheckResult` + `format_report`) so
  downstream automation could parse it; nicely tested.

## Coverage notes

- Did not run `libre-cr start` against a real daemon (per instructions).
- Did read every file under `crates/libre-cr-cli/src/`, plus
  `tests/integration.rs`, both CI workflows, both README files, CONTRIBUTING,
  `.gitignore`, the binding spec, the relevant Phase 7/8 plan sections, and
  both daemons' `cli.rs` to confirm the cross-crate config-path and pairing
  claims.
- `libre-cr --help` ran successfully and matches the spec's wrapper surface.
- No code was modified.
