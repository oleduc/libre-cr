# Extension Recipes

Concrete how-tos for the seams the architecture deliberately left open. Every
recipe ends in tests and (where wire-visible) a spec edit — see the
wire-change protocol in [conventions.md](conventions.md).

## Add a code-daemon MCP tool

Spec: [`specs/03-code-daemon.md`](../../specs/03-code-daemon.md) § Tool catalog.

1. **Implement the `Tool` trait** (`crates/libre-cr-code/src/tools/registry.rs`)
   in the matching module under `crates/libre-cr-code/src/tools/` —
   `git_tools.rs`, `grep_tools.rs`, `fs_tools.rs`, `repo_tools.rs`, … Each
   tool is a unit struct with `name`, `description`, `input_schema` (JSON
   Schema; the registry's `validate_against` enforces `required` keys and
   primitive types before your `call` runs), and `call`.
2. **Register it** in `build_registry()`
   (`crates/libre-cr-code/src/tools/mod.rs`). Registration order is the order
   tools are listed to clients.
3. **Arguments take `repo_path`/`repo_id`/`ref` — never `session_id`** (the
   ownership invariant; see [architecture.md](architecture.md)).
4. **Errors**: return `ToolError` with an `ErrorCode`
   (`crates/libre-cr-code/src/error.rs`). If no existing code fits, add the
   variant to `common::ErrorCategory` and `frames.ts` too — the strings must
   stay in lockstep.
5. **Tests**: unit tests in the module against a fixture repo (see
   `crates/libre-cr-code/src/util.rs` test helpers), plus a consumer-contract
   test in `crates/libre-cr-e2e/tests/mcp_consumer.rs` exercising the tool
   through a real spawned binary.

The review daemon picks the tool up automatically: it lists the code daemon's
schemas at startup and forwards calls through the `ToolRouter`
(`crates/libre-cr-review/src/tools/router.rs`). No review-daemon change
needed unless the tool requires injected context.

## Add an investigation verb

Spec: [`specs/06-investigation-verbs.md`](../../specs/06-investigation-verbs.md)
§ Adding A Verb.

> **Spec vs implementation:** the spec says "add an entry in `verbs.rs`";
> the file is actually `crates/libre-cr-review/src/verbs/mod.rs`.

1. Define a `pub const MY_VERB: Verb` in `verbs/mod.rs`: `id`, `label`,
   `description`, `required_selection` (`Any`/`File`/`Range`/`Symbol` —
   enforced server-side by `validate_selection`), the `system_prompt`
   addendum, and `suggested_tools`. Tune the prompt against a real PR before
   merging.
2. Add it to `CATALOG`. That's the whole registration:
   `GET /v1/verbs` is built from `catalog_descriptors()`, so the extension
   renders the new button with **zero extension changes** (it consumes the
   catalog dynamically — `VerbDescriptor` in
   `crates/libre-cr-common/src/http_api.rs`).
3. The agent loop appends `verb.system_prompt` to the base prompt
   (`build_system_prompt` in `crates/libre-cr-review/src/agent/loop_.rs`) —
   nothing to wire there.
4. **Tests**: snapshot/assembly test of the system prompt plus catalog and
   selection-requirement coverage in `verbs/mod.rs` tests, and a route-level
   test in `crates/libre-cr-review/tests/verbs_api.rs`.
5. Spec edit: add the verb to the catalog section of `specs/06`.

## Add a presentation tool

Spec: [`specs/09-presentation-tools.md`](../../specs/09-presentation-tools.md).
Five touchpoints, updated **in lockstep by hand** — there is no contract test
tying daemon and extension lists together (known erosion risk), so missing
one fails only at runtime:

1. **Daemon registry**: add the name to `PRESENTATION_TOOL_NAMES` and its
   schema to `presentation_tool_schemas()`
   (`crates/libre-cr-review/src/tools/presentation.rs`). The generic
   `presentation_call` WS frame carries any tool name, so
   `ws_frames.rs`/`frames.ts` only change if you need a new *frame* shape.
2. **Extension handler**: implement it in
   `extension/utils/presentation/handlers.ts` and add a case to
   `dispatchPresentationCall` (unknown tools answer
   `{ok:false, error:"unknown_tool"}`). Validate the input fields at runtime
   — the daemon's schema is advisory by the time it reaches the DOM.
3. **Frame/mute plumbing**: confirm the effect registers with the
   `PresentationManager` (`extension/utils/presentation/index.ts`) so
   clear-all and the mute gate cover it.
4. **Prompt guidance**: the tool list is named in `build_system_prompt`
   (`crates/libre-cr-review/src/agent/loop_.rs`) and in spec 09's prompt
   section — update both or the LLM will never call it.
5. **Tests**: handler unit tests in
   `extension/tests/presentation-handlers.test.ts`; daemon-side dispatch in
   `crates/libre-cr-review` presentation tests; ideally a browser E2E in
   `extension/e2e-browser/tests/presentation.spec.ts`.

## Add an LLM provider

1. Implement the `Provider` trait (`crates/libre-cr-review/src/provider/mod.rs`)
   in a new file beside `anthropic.rs` / `openai_compat.rs`. It yields
   `StreamEvent`s (text deltas, tool-use blocks, usage, stop reason).
2. Register the config `kind` string in the `build_provider` match
   (`provider/mod.rs`). API keys arrive encrypted (`api_key_enc`, decrypted
   with the install key) — never store plaintext.
3. Match the existing parsers' failure discipline: emit usage from wherever
   the API actually sends it, flush buffered tool state on early EOF, and
   surface `event: error` frames (see the `anthropic.rs` / `openai_compat.rs`
   tests for the expected shapes).
4. **Tests**: HTTP-level parser tests with recorded stream chunks, like
   `openai_compat.rs`'s `include_usage` tests. Providers are hot-swappable at
   runtime via `ProviderHandle` (`server/state.rs`) — no restart plumbing
   needed.

## Add a tree-sitter grammar (the Phase-1.1 seam)

`ast_search`, `list_symbols`, `find_definition`, `find_references` are
**stubs** today (`crates/libre-cr-code/src/tools/stubs.rs`) — registered with
their final names and schemas, returning `unsupported_language`. The seam:

- `crates/libre-cr-code/src/treesitter.rs`: `has_grammar()` (currently always
  `false`) and the `unsupported()` error constructor. Compile grammars in
  here; the planned set is spec 03 § "Phase B grammar set".
- `crates/libre-cr-code/src/languages.rs`: language detection.
- Replace a stub in `build_registry()` with the real impl, keeping the
  name and schema identical — the swap is wire-invisible to the review
  daemon and extension. Languages without a grammar keep returning
  `ErrorCode::UnsupportedLanguage` (surfaced to the LLM as a
  `ToolOutcome{ok:false}`, which typically routes it to `grep` instead —
  the turn never hard-fails).
- Phase C LSP backends slot in the same way: same names, backend chosen
  inside the tool impl per language/config.

## Add an HTTP route

1. **Typed response struct first** in
   `crates/libre-cr-common/src/http_api.rs` (`Serialize + Deserialize`,
   doc-comment naming the route).
2. Handler in `crates/libre-cr-review/src/server/routes.rs` returning
   `Json<YourResponse>`; wire it into the router in `server/mod.rs`. Errors
   are `crate::error::Error` — they already map to `ErrorEnvelope` +
   status codes.
3. Mirror the type in `extension/utils/daemon/frames.ts` and consume it via
   `extension/utils/daemon/client.ts`.
4. Integration test in `crates/libre-cr-review/tests/http_api.rs`; spec edit
   in `specs/04-review-daemon.md` § HTTP API.

## What NOT to do

The condensed guardrail list (full rationale in
[architecture.md](architecture.md) and the audit):

- **No PR/session/LLM awareness in the code daemon.** If your code-daemon
  change wants a PR URL or session id, it belongs in the review daemon.
- **No repo-filesystem access in the review daemon.** Everything goes
  through `CodeDaemonClient`.
- **No `json!` literal response bodies** in routes — typed structs in
  `http_api.rs`.
- **No new error-code strings** outside `common::ErrorCategory` (+ the
  `frames.ts` union).
- **No deepening the GitHub coupling**: don't extend `parse_pr_url`
  (`storage/store.rs`) or `utils/github/*` imports for platform-specific
  behavior — extract a platform adapter instead.
- **No new fields on the MCP `{ok, …}` envelope** without first typing it in
  `common` — today it's a string convention.
- **No second `ask` entry point that bypasses `BusyGuard`**
  (`server/ws.rs`): the single-flight-per-session invariant currently lives
  in the WS transport; route any new front door (e.g. Phase-4 MCP) through
  the same guard.
- **No schema changes outside numbered, forward-refusing migrations.**
- **No untracked `chrome.storage` keys** — extend `StorageShape`
  (`extension/utils/daemon/storage.ts`); think twice before adding
  per-session state there (the spec wants session state server-side).
- **No `sleep`-based synchronization in tests** — poll with a deadline.
