# Libre CR — Developer Documentation

Docs for people who build, test, extend, and review changes to Libre CR.
For usage docs see `../user-guide/`. For design intent see [`specs/`](../../specs/)
— the specs are the source of truth for *what* the system should do; these
pages explain how the implementation realizes it and where to make changes.

## Repo map

| Path | What it is |
|---|---|
| `specs/` | Design specs (01–09) + `plan.md` (phase plan). Spec edits accompany wire-format changes. |
| `crates/libre-cr-common/` | Shared wire contracts: WS frames, HTTP response types, error vocabulary, `PROTOCOL_VERSION`. |
| `crates/libre-cr-code/` | Code daemon: repo registry, worktrees, git/grep/fs tools, MCP server. Zero PR/LLM knowledge. |
| `crates/libre-cr-review/` | Review daemon: agent loop, LLM providers, verbs, HTTP/WS server, SQLite session store. |
| `crates/libre-cr-cli/` | `libre-cr` wrapper binary: supervisor, pairing, doctor, logs, update. |
| `crates/libre-cr-e2e/` | Spawn-real-binaries E2E suites (MCP consumer, HTTP consumer, supervision smoke). |
| `extension/` | WXT + React MV3 browser extension (TypeScript, strict). |
| `REVIEW/` | Code-review audit reports (round 1 and `round2/`); useful tribal knowledge. |
| `.github/workflows/` | `ci.yml` (merge gate) and `release.yml` (cross-compile stub). |

## The system in one paragraph

Libre CR is three cooperating components on the user's machine: a **browser
extension** that scrapes the GitHub PR page, captures selections, and renders
the Q&A panel; a **review daemon** (`libre-cr-review`) that owns sessions,
runs the LLM agent loop, and serves the extension over localhost HTTP +
WebSocket; and a **code daemon** (`libre-cr-code`) that owns local clones and
worktrees and exposes code-intelligence tools over MCP (stdio). The review
daemon is the only component that talks to an LLM; the code daemon is the
only component that touches repo filesystems. A thin wrapper CLI
(`libre-cr`) supervises the review daemon, which in turn supervises the code
daemon. Full detail (including the ownership invariant and failure handling):
[architecture.md](architecture.md).

## Quick links

- [setup.md](setup.md) — toolchain, first build, running daemons locally, extension dev loop
- [architecture.md](architecture.md) — the system as built; invariants; erosion risks
- [conventions.md](conventions.md) — Rust/TS style, wire-change protocol, migration and test rules
- [testing.md](testing.md) — every suite, every command, CI matrix
- [extending.md](extending.md) — recipes: new tool, verb, presentation tool, provider, route
- [release.md](release.md) — release pipeline state, versioning policy, compatibility

## Orientation tips

- Start with `specs/01-overview.md` and `specs/02-architecture.md`, then read
  `REVIEW/round2/03-architecture.md` — the audit's "confirmed sound" and
  "erosion risks" sections are the tribal knowledge this codebase runs on.
  (Several audit findings have since been fixed; [architecture.md](architecture.md)
  tracks which.)
- `specs/plan.md` defines the current phase. Out-of-phase work is fine but
  flag it in the PR.
- Anything that crosses a process boundary has a type in `libre-cr-common`
  (or should — see [conventions.md](conventions.md)).
