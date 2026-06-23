# Libre CR

A code review companion that augments GitHub pull request pages with
AI-assisted investigation. The reviewer drives; the assistant answers focused
questions about the code under review, grounded in the reviewer's **actual
local checkout** — full file contents, git history, cross-file structure —
not just the diff hunks visible in the browser.

Everything runs on your machine. The only network dependency is the LLM
provider you configure (Anthropic, OpenAI, or any OpenAI-compatible endpoint
including Ollama). No telemetry, no account, no GitHub tokens.

> **Status:** pre-release. Feature-complete against the v2 spec
> ([`specs/plan.md`](specs/plan.md) Phase B), installable from source.
> Package-manager distribution and store-published extensions are planned.

## How it works

```
 Browser extension          Review daemon              Code daemon
 (GitHub PR pages)   HTTP   (libre-cr-review)    MCP   (libre-cr-code)
 selection UI,      ◄─────► agent loop, LLM     ◄────► repo discovery,
 Q&A panel,          + WS   client, per-PR      stdio  worktrees, search,
 presentation               conversations              git history
 effects                    (SQLite)                   (also usable standalone
                                                        from any MCP client)
```

You select code in a PR diff and either click an **investigation verb**
(*Find callers · Show history · Related tests · Compare to base · Explain*)
or ask a free-form question. The daemon materializes the PR ref into a
managed git worktree, runs an agent loop with repo-aware tools, and streams
the answer back — optionally highlighting the lines it cites directly in the
diff. Findings you want to keep become severity-tagged notes, exported as a
Markdown review draft you paste into GitHub.

The code daemon is a standalone product: any MCP client (Claude Code, Claude
Desktop, custom agents) can use its 19 repo-intelligence tools without the
rest of the stack. See the [FAQ](docs/user-guide/faq.md).

## Documentation

| | Start here | Contents |
|---|---|---|
| **Using it** | [`docs/user-guide/`](docs/user-guide/README.md) | [Installation](docs/user-guide/installation.md) · [Configuration](docs/user-guide/configuration.md) · [Daily workflow](docs/user-guide/using.md) · [Troubleshooting](docs/user-guide/troubleshooting.md) · [FAQ](docs/user-guide/faq.md) |
| **Working on it** | [`docs/development/`](docs/development/README.md) | [Setup](docs/development/setup.md) · [Architecture as built](docs/development/architecture.md) · [Conventions](docs/development/conventions.md) · [Testing](docs/development/testing.md) · [Extending](docs/development/extending.md) · [Release](docs/development/release.md) |
| **Design intent** | [`specs/`](specs/01-overview.md) | The ten spec files that define the product; [`plan.md`](specs/plan.md) is the build sequence |
| **Contributing** | [`CONTRIBUTING.md`](CONTRIBUTING.md) | The short version + pointers |

## Quick start

```sh
# 1. Build (Rust stable + Node 20/pnpm 9 required)
cargo build --release
cd extension && pnpm install && pnpm build && cd ..

# 2. Start the daemons
export PATH="$PWD/target/release:$PATH"
libre-cr start            # runs in the foreground; use a second terminal below

# 3. Load the extension: chrome://extensions → Developer mode →
#    "Load unpacked" → extension/.output/chrome-mv3/

# 4. Pair (second terminal)
libre-cr pair             # prints a one-time code; paste it in the
                          # extension's options page with the endpoint

# 5. Configure your LLM key
libre-cr config           # opens the daemon's config UI in your browser
```

Open any GitHub PR, click the floating **CR** button, select some code, and
ask. Full walkthrough — including a mock-provider mode that needs no API
key — in the [installation guide](docs/user-guide/installation.md).

## Repository layout

```
crates/
  libre-cr-common/    Shared wire types: WS frames, HTTP responses, errors
  libre-cr-code/      Code daemon — standalone MCP server (repo intelligence)
  libre-cr-review/    Review daemon — agent loop, providers, sessions, HTTP/WS
  libre-cr-cli/       `libre-cr` wrapper CLI — supervision, pairing, doctor
  libre-cr-e2e/       End-to-end suites that spawn the real binaries
extension/            WXT + React browser extension (unit / e2e / e2e-browser)
specs/                Design specs (source of truth for behavior)
docs/                 User guide + developer documentation
REVIEW/               Multi-agent certification reports (round 1 and 2)
```

## Project principles

- **Human drives.** The LLM never annotates, summarizes, or reviews
  autonomously — it answers questions the reviewer asks.
- **Repo-grounded.** Answers come from tools run against a real checkout at
  the PR's ref, in an isolated worktree that never touches your working tree.
- **Local-first.** Conversations, notes, and config live in SQLite and TOML
  on your disk. The LLM provider is the only thing that ever sees code, and
  you choose it.
- **Composable.** MCP between the daemons means the code-intelligence layer
  is reusable by any agent, not locked to this product.

## License

MIT — see [LICENSE](LICENSE).
