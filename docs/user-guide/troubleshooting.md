# Troubleshooting

First moves for any problem:

```sh
libre-cr status    # is the daemon up? is /v1/health reachable?
libre-cr doctor    # git, binaries on PATH, file perms, endpoint sanity
libre-cr logs -f   # live-tail the review daemon, code daemon, and supervisor logs
```

## Daemon unreachable

| Symptom | Cause | Fix |
|---|---|---|
| Popup says "unreachable"; panel banner "Daemon offline" | Daemon not running | `libre-cr start` (it runs in the foreground — keep its terminal open, or wrap it in your own service manager) |
| Same, but `libre-cr status` says running | Port changed since pairing — the default config uses an OS-assigned port per start | Re-pair (`libre-cr pair` + options page) with the new endpoint from `~/.config/libre-cr/endpoint`, or set a fixed `server.port` in `review.toml` ([Configuration](configuration.md#server)) |
| `status` shows "stale PID" | Daemon crashed or machine rebooted | `libre-cr start` clears the stale PID file and relaunches |
| `start` exits with "crashed N times within 60s" | Review daemon failing at boot (bad config, port conflict) | `libre-cr logs` — look at the last lines of `libre-cr-review.log` |

## Pairing fails

| Symptom | Cause | Fix |
|---|---|---|
| `libre-cr pair`: "no daemon endpoint … Run `libre-cr start` first" | Daemon isn't running — codes are minted by the live daemon | Start it, then pair |
| Options page: unauthorized / invalid code | Code expired (~5 min TTL) or already used (single-use) | Run `libre-cr pair` again for a fresh code |
| Options page: rate limited (HTTP 429) | Too many failed redeem attempts from your machine | Wait ~60 seconds, mint a fresh code, retry |
| Pair succeeds but requests still fail with CORS errors | Extension was reloaded with a new extension ID (unpacked loads can change IDs) | Re-pair — pairing re-learns and persists the extension origin automatically |

## Worktree pending forever

The panel sits on "Preparing repo…" or errors out. The code daemon prepares worktrees by running `git fetch` in **your local clone**, using your normal git credentials.

| Symptom | Cause | Fix |
|---|---|---|
| Pending forever / `unknown_ref` error | Private repo: git can't authenticate non-interactively | In the repo, run `git fetch origin pull/<N>/head` yourself. If it prompts or fails, fix your credential helper / SSH agent — the daemon can't answer prompts |
| "Repo not found" style errors | Your checkout isn't under a discovery root | Add the parent directory of your clones to `discovery.default_roots` in `code.toml`, then `libre-cr restart` |
| Very slow first preparation | Cold fetch of a large repo is network-bound | Expected once per repo; subsequent PRs reuse the clone |
| Worktrees disappear over time | LRU eviction past the 5 GiB budget | Raise `worktrees.max_total_bytes` in `code.toml` if you want more retained |

## Provider errors

Errors during a turn appear inline in the Q&A turn with a Retry button.

| Error | Cause | Fix |
|---|---|---|
| 401 / authentication | Wrong or missing API key | Re-enter the key in the config UI (`libre-cr config`). Keys are stored encrypted; you can't fix them by editing `review.toml` by hand |
| 404 / model not found | Model name typo, or the configured `endpoint` doesn't serve that model | Check `provider.model` and `provider.endpoint` |
| 429 / rate limited | Provider-side quota | Wait, or switch model/provider |
| Timeouts / connection refused | Endpoint unreachable — wrong Ollama URL, VPN/proxy in the way | `curl` the endpoint yourself; Libre CR does no proxy auto-configuration |

## "I'm still getting mock answers"

The provider is still `kind = "mock"` (the out-of-the-box default), so every question replays the same scripted text — or completes instantly with nothing.

Fix: `libre-cr config`, set kind to `anthropic` or `openai_compat`, fill in model + key, save. Provider changes hot-reload — no restart needed. Verify with `curl -H "Authorization: Bearer $(cat ~/.config/libre-cr/token)" $(cat ~/.config/libre-cr/endpoint)/v1/config` — `provider.kind` should no longer be `mock`.

## Selector warnings banner

A soft warning listing GitHub page elements the scraper couldn't fully recognize (GitHub changed its markup). Q&A keeps working off the repo; selection/highlight precision may degrade. Update Libre CR; if it persists on the latest build, please file an issue with the listed selectors.

## Reading logs

```sh
libre-cr logs            # last 200 lines of all three logs
libre-cr logs -n 500     # more history
libre-cr logs -f         # follow
```

| File (in `~/.local/state/libre-cr/log/`) | What's in it |
|---|---|
| `libre-cr-review.log` | Agent loop, HTTP/WS, provider calls — start here for answer failures |
| `libre-cr-code.log` | Code daemon stderr: git fetches, worktree ops — start here for "preparing repo" issues |
| `supervisor.log` | Start/stop events, restart counts |

## `libre-cr doctor` interpretation

| Mark | Meaning |
|---|---|
| `✓` | Pass |
| `!` | Warning — works degraded or will bite later (e.g. binary not on PATH yet, token file perms wider than 0600) |
| `✗` | Failure — fix before continuing (e.g. git missing); doctor exits non-zero |
| `·` | Skipped — not applicable yet (files not created before first `start`) or not measured in this build (disk space) |

## Token rotation

There is no rotation flag yet (the spec'd `restart --rotate-token` is not implemented). Manual procedure:

```sh
libre-cr stop
rm ~/.config/libre-cr/token
libre-cr start        # a fresh token is generated
libre-cr pair         # the old extension credential is now invalid — re-pair
```

## Full reset

Wipe all state (conversations, notes, repos, worktrees, config) and start over:

```sh
libre-cr uninstall    # double confirmation; stops the daemon, removes config + data + state dirs
```

Or surgically: `libre-cr stop`, then delete any of `~/.config/libre-cr/`, `~/.local/share/libre-cr*/`, `~/.local/state/libre-cr/`.

## Uninstall

1. `libre-cr uninstall` (add `--force` to skip prompts) — stops the daemon and removes config, data, and logs.
2. Remove the binaries yourself — the command does **not** delete them (delete your clone's `target/release/` copies or wherever you put them on PATH).
3. Remove the extension from `chrome://extensions` / `about:debugging` as usual.
