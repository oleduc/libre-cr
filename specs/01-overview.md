# Libre CR — Overview

## What It Is

A code review companion that augments pull request pages with AI-assisted investigation. The reviewer drives; the assistant answers focused questions about the code under review, grounded in the user's actual local checkout.

The system has three components:

1. **Browser extension** — injects selection UI and a Q&A panel into GitHub PR pages.
2. **Review daemon** (`libre-cr-review`) — runs the agent loop, manages per-PR conversation state, orchestrates code-intelligence tools, hosts the LLM provider client.
3. **Code daemon** (`libre-cr-code`) — a standalone Rust MCP server for repo-aware code intelligence (symbol search, structural queries, git operations, worktree management). Usable on its own from any MCP client.

## Why a New Iteration

The previous iteration (`libre-cr-assistant`) was a working POC that delivered the wrong product. Two structural problems made it untenable:

- **LLM-as-orchestrator was the wrong model.** Functions that let the LLM autonomously annotate the diff produced output the reviewer had to verify rather than insight the reviewer could act on. Code review is a *human-driven* activity. The assistant should answer questions the reviewer asks, not preempt them.
- **Browser-only was the wrong surface.** Without access to the actual repo on disk — full file contents, git history, cross-file references — the assistant could only reason about the diff hunks visible in the DOM. Real code review requires reading code *outside* the diff.

This iteration replaces the LLM-orchestrated function model with a human-driven Q&A model, and replaces browser-only with a browser-plus-daemon architecture.

## Core Principles

- **Human drives.** The LLM is invoked when the reviewer asks. It does not autonomously decide what to highlight, annotate, or summarize.
- **Repo-grounded.** Every answer is rooted in the user's local checkout — actual file contents, git history, cross-file structure — not just the diff hunks.
- **Answers can demonstrate themselves.** When it helps, the LLM can highlight the line it's citing, scroll the diff to a location, or surface a link — through a bounded set of *presentation tools*. These amplify the answer; they do not replace it, and they never run without a user-initiated question. See `09-presentation-tools.md`.
- **Local-first.** All code intelligence and conversation state stays on the user's machine. The LLM provider is the only network dependency, and the user configures it. Configuration happens in the daemon's own config UI; the API key never leaves the daemon. For users who already have an `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` in their environment, the daemon can use it directly so there is no key to paste at all (see below).
- **Composable via MCP.** The code daemon is a standalone MCP server with value beyond our review extension. Other MCP clients (Claude Code, Claude Desktop, IDE plugins, future surfaces) can use it directly.
- **No GitHub tokens required.** PR content is read via DOM scraping in the extension, the same approach the POC validated. Posting back to GitHub is optional and deferred to a later phase.
- **Native by default.** The daemons are single Rust binaries. No Python, Node, or other runtime required to use the product.

## Killer Workflow

The product is shaped around one workflow:

1. Reviewer opens a PR in the browser.
2. The review daemon discovers the local checkout, silently materializes the PR ref into a managed worktree.
3. Reviewer selects code in the diff (a line, a range, a symbol).
4. A Q&A panel attached to the selection offers:
   - A set of **investigation verbs** as preset actions: *find callers*, *show history*, *related tests*, *compare to base*, *explain*.
   - A free-form question box.
5. The review daemon runs the agent loop. The LLM uses code-daemon tools to gather context and produces an answer that streams back to the panel.
6. Every interaction is recorded as a per-PR conversation. The reviewer can revisit it, edit it, and export it as a review draft when ready.

The verbs are not magic. They are well-tuned prompts that drive the same agent loop the free-form question box does. Anything a verb can do, the reviewer can do by asking.

## What Carries Over From the POC

- **GitHub DOM scraping** (`platform/github/`) — actually works against current GitHub markup; ports into the new extension as-is.
- **Shadow DOM UI shell** — theme detection, floating widget pattern, drag/resize/tile hooks. The Q&A panel is a floating widget.
- **Messaging type discipline** — the contract pattern between extension and background carries forward as the contract between extension and review daemon.
- **CSP-safe schema validator** — useful inside the extension for validating daemon responses.
- **`UIController` implementations** — the highlight/annotate/scroll/navigate DOM machinery from the POC powers the new presentation-tools layer (`09-presentation-tools.md`). The tool-callable contract shape is replaced; the underlying functions carry over.

## What Is Replaced

- The function/command runtime, registry, and built-in functions (`functions/runtime.ts`, `functions/builtin/*`) — the LLM-as-orchestrator pattern they implement is the thing being abandoned.
- Background-script LLM calls — the LLM lives in the review daemon now.
- The `UIController` *contract*: the LLM no longer drives the extension directly from inside the page. Presentation calls now route through the review daemon, over the WS channel, to a presentation handler in the extension. Same underlying effects; different control path.

## Tech Stack

| Layer | Choice | Why |
|---|---|---|
| Daemons | Rust | Single static binary, brew/scoop distribution, performance, long-running stability |
| Browser extension | WXT + React + TypeScript | Carry over from POC; cross-browser, Manifest V3 |
| Daemon ↔ extension transport | Localhost HTTP + WebSocket + token | Simple, debuggable, supports streaming |
| Daemon ↔ daemon protocol | MCP (stdio) | Standard, lets external clients reach the code daemon too |
| Conversation storage | SQLite (in review daemon) | Local, durable, queryable |
| Code intelligence (phase B) | ast-grep + ripgrep + tree-sitter + gitoxide | Native Rust, fast, no external runtime |
| Code intelligence (phase C) | + LSP client (`async-lsp` / `lsp-types`) | Semantic accuracy for cross-file references |
| LLM providers | Anthropic + OpenAI-compatible (incl. Ollama) | Carries forward the POC's provider model |

### LLM provider and credentials

The user picks a provider in the daemon's config UI (`/config-ui`). Three provider kinds are built in:

- **`mock`** — no network; canned responses for local development and tests.
- **`anthropic`** — official Messages API.
- **`openai_compat`** — OpenAI-compatible chat completions (api.openai.com, OpenRouter, Ollama, any compatible endpoint).

Credential resolution for `anthropic` / `openai_compat`: a key saved through the config UI (stored encrypted) always wins; if none is saved, the daemon falls back to the standard ambient environment variable (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`). The config UI reports which ambient credentials it detected so the user can leave the key field blank. For `anthropic`, the config UI can also fetch the live model list from the provider so the user picks a model instead of typing an id.

## Phased Plan At A Glance

- **Phase B (v2):** Browser extension + review daemon + code daemon, no LSP. The code daemon uses ast-grep, ripgrep, tree-sitter, and gitoxide. Investigation verbs ship as defined here. Conversation state persisted. Review export as a clipboard-pasteable draft.
- **Phase C (v2.5):** LSP support added to the code daemon. Tool implementations gain LSP-backed alternatives. No new MCP surface — same tools, better answers when an LSP is configured. Architectural seams for this are designed into phase B.
- **Later:** Optional GitHub OAuth for posting reviews directly. GitLab / Bitbucket adapters. IDE-side MCP clients beyond what external MCP-compatible tools already provide.

See `plan.md` for the full sequencing.

## Non-Goals

These are explicit non-goals for this iteration:

- **No autonomous review.** The product does not annotate, summarize, or "do a review" without being asked. Verbs are user-initiated. Presentation tools (`09-presentation-tools.md`) run only as part of answering a user question, never autonomously.
- **No code editing or rewriting.** The product reads code and answers questions. It does not propose patches, edit files, or open PRs.
- **No CI integration.** The product is a developer-side tool, not a CI gate.
- **No team-shared state on day 1.** Each user has their own daemon and their own conversation history. Team conventions belong in the global instructions block, not in the daemon's storage layer.
- **No bundled language servers.** If a phase-C LSP integration is added, users install LSPs themselves (which they almost always already have for their IDE).

## Spec Map

| File | Topic |
|---|---|
| `01-overview.md` | This file |
| `02-architecture.md` | Component diagram, data flow, ownership boundaries |
| `03-code-daemon.md` | `libre-cr-code` design |
| `04-review-daemon.md` | `libre-cr-review` design |
| `05-browser-extension.md` | Extension internals, transport, what carries over |
| `06-investigation-verbs.md` | Verb catalog + prompt design |
| `07-conversation-and-notes.md` | Persistence model, export flow |
| `08-distribution.md` | Install, supervision, updates |
| `09-presentation-tools.md` | LLM-dispatched browser actions (highlight, annotate, scroll, link) |
| `plan.md` | Phased implementation |
