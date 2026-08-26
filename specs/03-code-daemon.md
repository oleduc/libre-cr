# Code Daemon (`libre-cr-code`)

## Purpose

A standalone MCP server exposing repo-aware code intelligence over the Model Context Protocol. Its scope is everything that operates on a local git repository: structural and textual search, git history, worktree management, and (in phase C) LSP-backed semantic queries.

It has no awareness of pull requests, conversations, or LLMs. Every tool takes a `repo_path` (or a `repo_id` that resolves to one). It is usable by any MCP client without ever installing the review daemon or the browser extension.

## Process Model

- A single Rust binary: `libre-cr-code`.
- Two modes:
  - **stdio mode (default):** `libre-cr-code mcp-stdio` — speaks MCP on stdin/stdout. The expected way for parents (review daemon, Claude Code, Claude Desktop) to spawn it.
  - **socket mode:** `libre-cr-code mcp-socket --path /tmp/libre-cr-code.sock` — speaks MCP over a Unix domain socket. Lets a single long-running instance be shared across multiple parents.
- The binary is also a CLI for direct use: `libre-cr-code discover-repo <url>`, `libre-cr-code prepare-worktree …`, useful for shell users and for debugging.

## Tool Surface (Phase B)

All tools are exposed via the MCP `tools/list` and `tools/call` methods. Tool names use `snake_case`. Every tool returns a JSON object with at minimum `{ ok: bool, error?: string }`; success payloads are tool-specific.

### Repo and worktree management

- **`discover_repo`** `{ remote_url: string }` → `{ repo_id, repo_path, default_branch }` or `{ ok: false }`
  Look up a previously registered repo by remote URL. Returns the local checkout path if found.

- **`scan_for_repos`** `{ roots: string[] }` → `{ discovered: [{ repo_id, repo_path, remotes: string[] }] }`
  Walk the given root directories (default: configured roots), find git repos, register them. Returns what it added. Idempotent.

- **`clone_repo`** `{ remote_url: string, target_dir?: string }` → `{ repo_id, repo_path }`
  Clone the given repo into `target_dir` (defaults to managed cache: `~/.local/share/libre-cr-code/repos/<owner>/<repo>`).

- **`prepare_worktree`** `{ repo_id: string, ref: string, name?: string }` → `{ worktree_path }`
  Fetch `ref` (e.g., `pull/123/head`) and materialize it as a worktree under `~/.local/share/libre-cr-code/worktrees/<repo_id>/<name|sanitized-ref>`. Idempotent — returns the existing worktree if already prepared. Tracks last-used timestamp for LRU eviction.

- **`list_worktrees`** `{ repo_id?: string }` → `{ worktrees: [{ repo_id, ref, path, last_used_at }] }`

- **`remove_worktree`** `{ worktree_path: string }` → `{ ok }`

### File and structural reads

All file-reading tools take an optional `ref` argument. If omitted, they read from the working tree at `repo_path`. If provided, they read from the git object database at that ref (no checkout needed).

- **`read_file`** `{ repo_path: string, file: string, ref?: string, start_line?: number, end_line?: number }` → `{ content, total_lines }`
  Returns file content, optionally sliced. `ref` lets the caller read any commit's version.

- **`list_dir`** `{ repo_path: string, dir: string, ref?: string, recursive?: bool, max_depth?: number }` → `{ entries: [{ name, kind: "file"|"dir", size? }] }`

- **`stat_file`** `{ repo_path: string, file: string, ref?: string }` → `{ size, language, is_binary }`
  Cheap metadata read. `language` inferred from extension + content-based fallback.

### Search

- **`grep`** `{ repo_path: string, pattern: string, ref?: string, paths?: string[], glob?: string, fixed_string?: bool, max_matches?: number }` → `{ matches: [{ file, line, column, content }], truncated: bool }`
  ripgrep-backed text search. `paths` limits to a set of files/dirs; `glob` filters by pattern. Defaults: regex, max 200 matches, then `truncated: true`.

- **`ast_search`** `{ repo_path: string, language: string, pattern: string, ref?: string, paths?: string[] }` → `{ matches: [{ file, range: { start_line, end_line, start_col, end_col }, captured_text }] }`
  ast-grep structural search. `pattern` is in ast-grep's pattern syntax (e.g., `foo($A)` matches any call to `foo` with one argument). Required input for "find callers" and similar.

### Symbols (phase B: AST-derived)

- **`find_definition`** `{ repo_path: string, file: string, line: number, column: number, ref?: string }` → `{ definitions: [{ file, range, symbol_name, kind, confidence }] }`
  Phase B: tree-sitter-based — finds the identifier at the cursor, then ast-grep-searches the repo for definitions matching the identifier's name and kind. **Name-based, not semantic** (see Language Support below). Each result includes a `confidence` field: `high` (unique name + matching kind), `medium` (one of a few candidates), `low` (common name with many candidates). Phase C: routes to LSP `textDocument/definition` when available; confidence becomes `lsp` (semantic).

- **`find_references`** `{ repo_path: string, file: string, line: number, column: number, ref?: string }` → `{ references: [{ file, range, context_line, confidence }] }`
  Phase B: name-based AST search. Same `confidence` model as `find_definition`. Will return false positives for overloaded names (two unrelated functions named `init`). Phase C: LSP `textDocument/references`.

- **`list_symbols`** `{ repo_path: string, file: string, ref?: string }` → `{ symbols: [{ name, kind, range, parent? }] }`
  Tree-sitter tags query per language. Useful for "outline this file" and for the agent to orient itself before grepping.

### Git operations

- **`git_log`** `{ repo_path: string, file?: string, ref?: string, max_count?: number, since?: string }` → `{ commits: [{ sha, author, date, summary }] }`

- **`git_blame`** `{ repo_path: string, file: string, ref?: string, start_line?: number, end_line?: number }` → `{ lines: [{ line, sha, author, date, summary }] }`

- **`git_show`** `{ repo_path: string, sha: string, file?: string }` → `{ message, author, date, diff }`
  Full commit info; if `file` is given, only that file's diff.

- **`git_diff`** `{ repo_path: string, from_ref: string, to_ref: string, paths?: string[] }` → `{ files: [{ path, status, hunks: [{ … }] }] }`
  Structured diff between two refs. Used by review-daemon-side tooling to confirm PR diff matches what the extension scraped.

### Language detection

- **`detect_languages`** `{ repo_path: string }` → `{ languages: [{ language, file_count, line_count }] }`
  Cheap project-level shape. Useful for the agent to know what it's looking at before formulating queries.

## Language Support

The daemon's tools fall into two coverage classes: **language-agnostic** (work on any file) and **language-specific** (require a tree-sitter grammar for the file's language).

### Coverage by tool

| Tool | Requires | Works for |
|---|---|---|
| `read_file`, `list_dir`, `stat_file` | nothing | any file |
| `grep` | nothing | any text file |
| `git_log`, `git_blame`, `git_show`, `git_diff` | nothing | any tracked file |
| `detect_languages` | nothing (extension + content heuristics) | any repo |
| `ast_search` | grammar | listed languages |
| `list_symbols` | grammar + `tags.scm` query | listed languages |
| `find_definition`, `find_references` | grammar + name-based query (Phase B) or LSP (Phase C) | listed languages |
| `clone_repo`, `discover_repo`, `prepare_worktree`, `list_worktrees`, `remove_worktree` | nothing | any repo |

### Phase B grammar set

These languages are compiled in to `libre-cr-code` by default. All AST-aware tools work for them:

| Language | Grammar crate | Notes |
|---|---|---|
| Rust | `tree-sitter-rust` | |
| Go | `tree-sitter-go` | |
| JavaScript | `tree-sitter-javascript` | includes JSX |
| TypeScript | `tree-sitter-typescript` | includes TSX |
| Python | `tree-sitter-python` | |
| Java | `tree-sitter-java` | |
| C | `tree-sitter-c` | |
| C++ | `tree-sitter-cpp` | |
| Ruby | `tree-sitter-ruby` | |
| PHP | `tree-sitter-php` | |
| Bash | `tree-sitter-bash` | useful for scripts, CI configs, hooks |

For any other language, the language-agnostic tools still apply — the agent can grep, read, and follow git history. Only the AST-aware tools become unavailable.

### Phase B is name-based, not semantic

This is the most important caveat in the whole spec to communicate honestly, because it affects how the LLM should reason about results.

`find_definition` and `find_references` in Phase B do **name-based AST search**: they identify the symbol's lexical name and kind (function, method, type, etc.) and look for AST nodes matching that name and kind across the repo. This is what tree-sitter and ast-grep give us without language servers.

What this gets right:

- A function `validateUserInput` defined once, called in five places — all five references found.
- A type `UserId` referenced in struct fields and method signatures — found.
- A unique top-level constant — found.

What this gets wrong:

- **Overloaded names.** Two unrelated methods named `init`, both will appear as references to either.
- **Dynamic dispatch.** `obj[name]()` in JavaScript, reflection in Java, `getattr(o, "x")` in Python — invisible.
- **Polymorphism / interface dispatch.** A method `fn save()` implemented by ten types — `find_references` on one implementation can't distinguish call sites that go through the interface from those that target that specific type.
- **Aliasing.** `import { foo as bar }` or `use foo as bar` — references to `bar` won't be linked to `foo`'s definition without explicit handling. Phase B implements aliasing only for the most common patterns per language.
- **Macros and codegen.** A symbol synthesized by a macro is invisible to AST search.

Every result from these tools carries a `confidence` field:

- `high` — the name is unique in the visible codebase (single defining occurrence, all references match name + kind).
- `medium` — multiple candidates found; the result set is filtered to the kind that matches the cursor's context.
- `low` — common name with many candidates; results are best-effort.

The agent's system prompt instructs it to honor confidence: report it to the reviewer, and follow up with grep or context reads to disambiguate medium/low results before making claims.

**Phase C with LSP fixes most of this.** When an LSP is configured for the file's language, the same tools route to `textDocument/definition` / `textDocument/references`, and `confidence` is reported as `lsp` (semantic). The Phase B implementations remain as a fallback when no LSP is available.

### Fast-follow languages (not in Phase B)

Grammars exist, integration cost is low, will likely land soon after v2:

- **C#** (`tree-sitter-c-sharp`) — common in enterprise.
- **Kotlin** (`tree-sitter-kotlin`) — Android codebases.
- **Swift** (`tree-sitter-swift`) — iOS codebases.
- **Scala** (`tree-sitter-scala`) — JVM backends.

These don't ship in v2 to keep the binary lean and the test surface small, not because of any technical objection.

### Long-tail languages

Tree-sitter grammars exist for Dart, Elixir, Haskell, OCaml, Zig, Nix, Lua, GraphQL, SQL, HTML, CSS, YAML, JSON, TOML, Markdown, and others. Each is a `cargo add` + register away. We add them when a real review workflow demands it, not preemptively. Adding a grammar is a non-breaking, additive change to `libre-cr-code` — no protocol or schema impact.

## Tool Surface (Phase C additions)

When LSP backends are available for a repo's languages, the same tool names route to LSP-backed implementations transparently. The MCP tool surface does not change. New tools added in phase C:

- **`hover`** `{ repo_path, file, line, column, ref? }` → `{ kind, signature, doc? }`
- **`type_at`** `{ repo_path, file, line, column, ref? }` → `{ type }`
- **`workspace_symbols`** `{ repo_path, query }` → `{ symbols: [{ name, kind, location }] }`
- **`call_hierarchy`** `{ repo_path, file, line, column, direction: "callers"|"callees" }` → `{ items: [{ name, location }] }`

LSP integration is detailed in `plan.md` Phase C section; the architectural seam is described below.

## Internal Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                         libre-cr-code                         │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  MCP Server (rmcp / hand-rolled)                       │  │
│  │  • stdio transport                                     │  │
│  │  • Unix socket transport                               │  │
│  │  • tools/list, tools/call, prompts/list (unused)       │  │
│  └────────────────────────┬───────────────────────────────┘  │
│                           │                                   │
│  ┌────────────────────────▼───────────────────────────────┐  │
│  │  Tool dispatcher                                       │  │
│  │  ─ Maps tool name → registered handler                 │  │
│  │  ─ Validates input schema                              │  │
│  │  ─ Wraps errors into MCP error envelope                │  │
│  └────────────────────────┬───────────────────────────────┘  │
│                           │                                   │
│         ┌─────────────────┼──────────────────┐                │
│         ▼                 ▼                  ▼                │
│   ┌──────────┐      ┌──────────┐       ┌──────────┐           │
│   │   Repo   │      │  Search  │       │   Git    │           │
│   │ registry │      │ backends │       │ backend  │           │
│   │ (SQLite) │      │          │       │(gitoxide)│           │
│   └──────────┘      └────┬─────┘       └──────────┘           │
│                          │                                    │
│              ┌───────────┼───────────┐                        │
│              ▼           ▼           ▼                        │
│         ┌─────────┐ ┌─────────┐ ┌────────────┐                │
│         │ripgrep  │ │ast-grep │ │tree-sitter │                │
│         │ (lib)   │ │ (lib)   │ │  (lib +    │                │
│         │         │ │         │ │  grammars) │                │
│         └─────────┘ └─────────┘ └────────────┘                │
│                                                               │
│                                Phase C:                       │
│              ┌────────────────────────────────────┐           │
│              │  LSP backend (trait + impls)       │           │
│              │  ─ rust-analyzer, gopls, tsserver, │           │
│              │    pyright, …                      │           │
│              │  ─ Lifecycle: spawn-on-demand,     │           │
│              │    idle-timeout shutdown           │           │
│              └────────────────────────────────────┘           │
└──────────────────────────────────────────────────────────────┘
```

### Tool registration

Every tool is a Rust struct implementing:

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> &serde_json::Value;
    async fn call(&self, ctx: &ToolContext, input: serde_json::Value)
        -> Result<serde_json::Value, ToolError>;
}
```

`ToolContext` carries handles to the repo registry, the git backend, the search backends, and (phase C) the LSP backend registry. Tools are registered at process start; the MCP server reflects them via `tools/list`.

This is the seam phase C plugs into: `find_definition`'s tool struct internally asks the LSP backend registry for a backend for the file's language; if one exists and is healthy, it routes there; otherwise it falls back to the AST implementation. Same tool, different implementation chosen per call.

### Repo registry (SQLite)

Schema:

```sql
CREATE TABLE repos (
  repo_id      TEXT PRIMARY KEY,        -- "github.com/foo/bar" (canonicalized)
  local_path   TEXT NOT NULL,
  registered_at INTEGER NOT NULL,
  last_used_at  INTEGER NOT NULL
);

CREATE TABLE repo_remotes (
  repo_id      TEXT NOT NULL,
  remote_url   TEXT NOT NULL,            -- normalized
  PRIMARY KEY (repo_id, remote_url),
  FOREIGN KEY (repo_id) REFERENCES repos(repo_id)
);

CREATE TABLE worktrees (
  worktree_path TEXT PRIMARY KEY,
  repo_id       TEXT NOT NULL,
  ref_name      TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  last_used_at  INTEGER NOT NULL,
  FOREIGN KEY (repo_id) REFERENCES repos(repo_id)
);
```

Remote URLs are canonicalized:
- `git@github.com:foo/bar.git` → `github.com/foo/bar`
- `https://github.com/foo/bar.git` → `github.com/foo/bar`
- Case-folded, `.git` stripped.

`discover_repo` queries by canonicalized remote URL.

The registry database carries a `_schema_version` table with versioned, forward-refusing migrations — the same pattern the review daemon's storage uses. The daemon refuses to open a database stamped with a newer schema version than it knows about.

### Worktree management

- **Location:** `~/.local/share/libre-cr-code/worktrees/<repo_id>/<sanitized-ref>`.
- **Creation:** `git fetch <ref>` against the canonical remote, then `git worktree add --detach <path> FETCH_HEAD`. Detached HEAD because we have no intention of letting the user commit into a managed worktree.
- **Reuse:** `prepare_worktree` is idempotent but never serves stale code. When the worktree already exists it still re-fetches the ref and, if the worktree's `HEAD` diverges from the freshly fetched tip (e.g., the PR head was force-pushed), it `git reset --hard`s the worktree to `FETCH_HEAD`. The fetch is skipped only when the caller passes an `expected_sha` that already matches. Updates `last_used_at`. This guarantees the reviewer reads the current PR head rather than a cached older revision while the extension's "PR diff changed" banner is showing.
- **Eviction:** Background task wakes hourly. If total worktree disk usage > threshold (default 5 GB, configurable), evict by `last_used_at` ascending until under threshold. Eviction = `git worktree remove --force`.
- **Cleanup on parent death:** Worktrees persist across daemon restarts. They're real on-disk artifacts the user might want to inspect. The daemon does not delete them at shutdown.

### Search backends

- **ripgrep:** invoked as a library via the `grep` crate. ~No process spawn cost.
- **ast-grep:** invoked as a library via `ast_grep_core`. Tree-sitter parsing is the dominant cost; we cache parsed ASTs per `(file_path, mtime)` in an LRU.
- **tree-sitter:** language grammars compiled in. See **Language Support** below for the Phase B set. Additional grammars are an additive change.

### Git backend

- **gitoxide** for read operations (`log`, `blame`, `show`, `diff`, `ls-tree`).
- **`git` CLI fallback** for `fetch` and `worktree add` — gitoxide's coverage of these is improving but uneven across versions. Spawning the git CLI is acceptable for these slow operations (network-bound or filesystem-bound anyway).
- The git CLI dependency is documented but not bundled. We assume any developer has git.

## Configuration

Config file: `~/.config/libre-cr/code.toml`. Created on first run with defaults. All wrapper-managed config lives under the single `~/.config/libre-cr/` namespace (see `08-distribution.md` § Configuration Layout). The legacy path `~/.config/libre-cr-code/config.toml` is still recognized: if only the legacy file exists, the daemon migrates it into the shared namespace once on load. Data and state still live under `~/.local/share/libre-cr-code/` — only the config file moved.

```toml
[storage]
data_dir = "~/.local/share/libre-cr-code"
state_db = "~/.local/share/libre-cr-code/state.db"

[worktrees]
# LRU eviction threshold across all worktrees, in bytes
max_total_bytes = 5_368_709_120  # 5 GB
eviction_check_interval_secs = 3600

[discovery]
# Roots scanned by scan_for_repos when called with no args
default_roots = ["~/code", "~/Dev", "~/src"]

[grep]
default_max_matches = 200

[ast_grep]
# Per-language AST cache size (number of parsed files held in LRU)
ast_cache_size = 256

# Phase C only — empty in phase B
[lsp]
# enabled = false in phase B
# Per-language overrides; if a binary is on PATH it's auto-detected.
# rust = { command = "rust-analyzer" }
# typescript = { command = "typescript-language-server", args = ["--stdio"] }
```

## CLI Surface

For shell users and debugging:

```
libre-cr-code mcp-stdio                          # speak MCP on stdio (default for parents)
libre-cr-code mcp-socket --path <socket>         # speak MCP over Unix socket
libre-cr-code scan [--roots ~/code,…]            # scan + register
libre-cr-code discover <remote-url>              # print local path or exit nonzero
libre-cr-code prepare <repo-id> <ref>            # prepare a worktree, print path
libre-cr-code worktrees                          # list all worktrees + LRU stats
libre-cr-code evict --dry-run                    # show what eviction would remove
libre-cr-code tools                              # list all MCP tools and their schemas
libre-cr-code config edit                        # open config in $EDITOR
libre-cr-code doctor                             # check git CLI, grammars, disk space
```

## Performance Budget

Per-tool soft targets, for medium-sized repos (~100k LOC):

| Tool | Target | Notes |
|---|---|---|
| `read_file` | <10 ms | filesystem |
| `grep` (200-match cap) | <100 ms | ripgrep, parallel |
| `ast_search` | <500 ms | first call per file pays AST parse; cached after |
| `find_references` (phase B) | <2 s | AST-derived name matching |
| `find_references` (phase C, LSP) | <500 ms after warm-up | LSP index assumed warm |
| `list_symbols` | <50 ms | tree-sitter tags, single file |
| `git_log` (max 100) | <100 ms | gitoxide |
| `git_blame` (200 lines) | <500 ms | gitoxide |
| `prepare_worktree` (cached) | <100 ms | filesystem only |
| `prepare_worktree` (cold) | seconds to minutes | network-bound `git fetch` |

These are targets, not guarantees. Repos differ. The daemon logs every tool call with timing; we tune from real data.

## Concurrency

- All tools are async (`tokio`). Multiple MCP clients can issue tool calls concurrently.
- **Requests on a single MCP connection are dispatched concurrently**, bounded by a semaphore (`MAX_CONCURRENT_REQUESTS`), so one slow tool call (e.g. a minutes-long cold `prepare_worktree` fetch) does not head-of-line-block every later request on the same connection. A single writer task serializes the response bytes so concurrent handlers never interleave their output. The MCP `initialize` handshake is the one exception: it is answered inline before any later request is spawned.
- Git subprocesses (`fetch`, `worktree add`, `reset`, `rev-parse`) run via `tokio::process` so they never block a tokio worker thread.
- The repo registry uses a single SQLite connection guarded by a mutex (writes are infrequent; reads are quick). Upgrade to a connection pool if needed.
- The AST cache and language servers (phase C) use per-language locks; a slow Python analysis doesn't block a Rust query.
- Worktree creation for the same `(repo_id, ref)` is single-flighted: concurrent `prepare_worktree` calls share a single in-flight fetch. The single-flight map holds `Weak` references and is pruned on every acquire, so it stays bounded by the number of *in-flight* prepares rather than growing one entry per ref ever requested.

## Error Model

Tool errors return `{ ok: false, error: "<machine-readable code>", message: "<human text>", details?: {...} }`.

Error codes (extensible):

- `unknown_repo` — `repo_id` doesn't resolve.
- `unknown_ref` — ref couldn't be fetched (404, no permission, network).
- `worktree_busy` — another caller is preparing the same worktree (caller should retry).
- `unsupported_language` — operation requires a grammar/LSP not configured.
- `not_in_workspace` — file is outside the repo.
- `validation_failed` — malformed tool input (bad/missing arguments, schema mismatch). This code is the shared envelope vocabulary across the daemons and the extension; it is the same string as `libre_cr_common::ErrorCategory::ValidationFailed`.
- `internal` — bug or unexpected condition; details include a backtrace in debug builds.

The review daemon translates these into user-facing messages with retry/recover suggestions.

## Logging and Telemetry

- Structured logs via `tracing`. Default level `info`; `RUST_LOG=debug` for verbose.
- Per-tool spans with `tool_name`, `repo_id`, `latency_ms`, `result: ok|err`.
- **No outbound telemetry.** Logs are local files only: `~/.local/state/libre-cr-code/log/<date>.log`. Rotated daily, kept 14 days.

## Security Posture

- The daemon reads anywhere the user's process can read. It can read SSH keys and other secrets if asked. This is acceptable for a local dev tool; we are not a sandboxed runtime.
- The MCP server has no authentication of its own. Trust model: the parent that spawned the stdio/socket connection is trusted. Unix socket permissions (0600, owned by user) gate access.
- Tools that take a `file` argument resolve relative to `repo_path` and reject paths that escape it via `..` or symlinks pointing outside. Defense in depth against accidents, not a sandbox.
- `clone_repo` writes only inside `data_dir`. The managed cache is never outside the configured root.

## Versioning and Compatibility

- The binary version is the source of truth.
- The MCP tool surface is the public contract. Adding tools is non-breaking. Removing or changing tool signatures bumps the major version.
- The SQLite schema has a migration table; the daemon refuses to start against a DB newer than itself.
- Config files are forward-compatible — unknown keys are ignored with a warning.

## Standalone Use Cases (Beyond Our Review Extension)

Because the code daemon knows nothing about PRs, it's a useful general-purpose MCP server. Three example flows we explicitly support:

1. **Claude Desktop user wants to ask questions about a local repo.** Configures `libre-cr-code mcp-stdio` as an MCP server in Claude Desktop. Calls `scan_for_repos` once, then uses any tool with the resulting `repo_id`.
2. **Claude Code session in a repo.** `libre-cr-code` registered as an MCP server. Claude Code's agent uses our richer tools (`ast_search`, `find_references`, `git_blame`) instead of shelling out to grep/git.
3. **Custom agent wants repo-aware code intelligence.** Any Rust/Python/TS agent that speaks MCP can use `libre-cr-code` without our review daemon ever entering the picture.

These flows justify treating `libre-cr-code` as a real product, not internal infrastructure. The standalone documentation lives alongside this spec (deferred to the implementation phase).
