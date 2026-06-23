# Project Conventions

Extracted from the codebase as it exists — when in doubt, imitate the nearest
neighbor. CI enforces the mechanical parts (see [testing.md](testing.md)).

## Spec-first

The specs in `specs/` are the source of truth for design intent. Code matches
the spec, not the other way around. If the spec is wrong, fix the spec (a
separate PR is fine). PR descriptions name the spec section they implement or
revise. `specs/plan.md` defines the current phase — out-of-phase work is
welcome but must be flagged.

## Rust style

- **Errors: `thiserror` enum per crate, shared vocabulary at boundaries.**
  Each crate has its own `error.rs` (`crates/libre-cr-review/src/error.rs`,
  `crates/libre-cr-code/src/error.rs`). At a process boundary every error is
  converted to the shared `common::ErrorEnvelope` with a
  `common::ErrorCategory` (review daemon: `Error::IntoResponse`; code daemon:
  `ErrorCode::as_str()` — its strings deliberately match `ErrorCategory`'s
  serde names). **Never invent a new error-code string**: add a variant to
  `crates/libre-cr-common/src/error.rs` *and* the `ErrorCategory` union in
  `extension/utils/daemon/frames.ts` in the same PR.
- **Tokio idioms in use** (copy these, don't improvise):
  - actor + oneshot-per-call for owned connections
    (`crates/libre-cr-review/src/code_daemon/client.rs`);
  - RAII guards for invariants (`BusyGuard` in `server/ws.rs`) — never
    manual insert/remove around an await point;
  - `RwLock<Arc<dyn …>>` handle for hot-swappable singletons
    (`ProviderHandle` in `server/state.rs`);
  - background work via `tokio::spawn` with status published to a board
    (`spawn_prepare` / `SessionStatusBoard` in `worktree.rs`);
  - parallel fan-out with stable ordering via `join_all`
    (`agent/loop_.rs`).
  - Known debt, not a pattern to copy: the code daemon does blocking
    git/gix work directly on the runtime (fine on serial stdio; flagged for
    `mcp-socket` mode).
- **Module layout**: a directory per subsystem with a `mod.rs`
  (`server/`, `tools/`, `storage/`, `provider/`, …). Binaries split
  `main.rs` (thin) from `cli.rs` (clap surface) and, where the crate is also
  a lib, `lib.rs`.
- **Logging**: `tracing` *events* with structured fields
  (`tracing::info!(tool_name = %name, latency_ms, …)` — see
  `crates/libre-cr-code/src/mcp/server.rs`), `RUST_LOG` env-filter, output to
  stderr. There is no span-per-turn convention; durable per-call telemetry
  goes to the `tool_traces` table instead. Module-level `//!` doc comments
  open every file and usually cite the spec section they implement.
- `cargo fmt` and `clippy --all-targets -- -D warnings` are the bar; CI runs
  both.

## TypeScript style

- **Strict mode, no `any`.** `extension/tsconfig.json` sets `strict` +
  `noUncheckedIndexedAccess` + `noFallthroughCasesInSwitch`. Use `unknown` +
  narrowing at boundaries.
- **Runtime validation at every wire boundary.** Incoming WS frames go
  through `parseServerFrame` (`extension/utils/daemon/frames.ts`), which
  drops unknown/malformed frames with a warning instead of crashing the
  panel. New frame types extend `SERVER_FRAME_TYPES` and the union together.
- **Inline styles, not Tailwind.** All panel CSS is a string constant
  injected into the Shadow DOM (`extension/components/styles.ts`) so nothing
  leaks to/from the host page and there is no bundler-side CSS pipeline.
- `chrome.storage` access only through the typed `StorageShape` keys in
  `extension/utils/daemon/storage.ts` — add a key there, never ad-hoc.
- Comments mirror Rust: each module opens with a header comment that names
  its Rust counterpart where one exists ("Mirror of
  `libre-cr-common::ws_frames`…").

## Wire-format change protocol

Any change to HTTP routes, WS frames, MCP tool signatures, or the error
vocabulary requires — **in the same PR**:

1. The spec edit (`specs/0X-….md`).
2. The Rust type in `crates/libre-cr-common/` (`ws_frames.rs`, `http_api.rs`,
   or `error.rs`). Route handlers return `Json<TypedStruct>`, never bare
   `json!` literals.
3. The mirror in `extension/utils/daemon/frames.ts` (the Rust side is the
   source of truth; the TS side describes the wire).
4. A `PROTOCOL_VERSION` bump (`crates/libre-cr-common/src/version.rs` **and**
   the constant in `frames.ts`) if the change is breaking. Additive,
   serde-defaultable changes don't bump — the compat promise is that minor
   versions stay wire-compatible (`specs/08-distribution.md` § Compatibility
   Pairing).

## Migration rules

Both SQLite stores (`crates/libre-cr-review/src/storage/migrations.rs`,
`crates/libre-cr-code/src/repo/registry.rs`) follow the same discipline:

- **Numbered**: `M0001`, `M0002`, … applied in order inside a transaction,
  each stamping `_schema_version`.
- **Additive only**: new tables or `ALTER TABLE … ADD COLUMN` with NULLable /
  defaulted columns. No destructive rewrites.
- **Forward-refusing**: a binary refuses to open a database stamped newer
  than its `SCHEMA_VERSION`. Bump the constant with your migration and keep
  the `refuses_newer_schema` test pattern.

## Test conventions

- **Unit tests live beside the code** in `#[cfg(test)] mod tests` /
  `*.test.ts(x)` next to what they test (extension unit tests sit in
  `extension/tests/`).
- **Integration tests** go in the crate's `tests/` dir
  (e.g. `crates/libre-cr-review/tests/http_api.rs` runs the axum app
  in-process).
- **E2E** (spawned real binaries) goes in `crates/libre-cr-e2e/` or
  `extension/e2e{,-browser}/` — never in a daemon crate.
- **Fixture builders, not checked-in repos**: git fixtures are constructed
  programmatically (`crates/libre-cr-e2e/tests/common/git_fixture.rs`);
  HTML fixtures for the browser suite are a single static page
  (`extension/e2e-browser/fixtures/pr-page.html`).
- **No sleep-as-sync.** Wait on observable conditions with a deadline
  (endpoint-file polling in `tests/common/spawned_daemon.rs`). Short sleeps
  are acceptable only as the poll interval inside such a loop.
- Mock everything nondeterministic via config, not monkey-patching: the mock
  provider replays `mock.provider_script`
  (`crates/libre-cr-review/src/config.rs`).

See [testing.md](testing.md) for which layer a new test belongs to.

## Commits and PRs

- One logical change per commit; keep commits focused.
- PR description states which spec section the change implements or revises.
- Wire-format changes follow the protocol above — reviewers reject a wire
  change whose spec edit or TS mirror is missing.
- The merge gate is CI (`.github/workflows/ci.yml`): fmt, clippy `-D
  warnings`, `cargo test --workspace` on Linux/macOS/Windows, extension
  typecheck + unit tests, and the Playwright browser suite.
