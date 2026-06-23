# Security review — libre-cr v2

## Summary

**Acceptable-with-fixes.** The architecture is sound and the surface area is
small: a 127.0.0.1-bound HTTP/WS daemon with bearer-token auth, AES-GCM-encrypted
provider key, AST-only worktree access, and a strict path traversal guard.
The known attacker preconditions (the user's machine is inside the trust
boundary) are respected. There are no remote-code-execution or full-auth-bypass
findings. Three issues need addressing before v2 GA — none critical, two of
medium importance, and a small set of defense-in-depth gaps.

## Threats considered

1. Localhost HTTP/WS surface (bearer + origin + CORS).
2. Pairing flow (one-time code generation, redemption, replay).
3. API key at rest (AES-GCM, key derivation, file mode).
4. Path traversal in `read_file` / `list_dir` / `stat_file`.
5. Subprocess invocation safety (`git fetch`, `git diff`, `git show`, `git blame`).
6. MCP wire integrity between review and code daemons.
7. LLM-supplied input as untrusted (DOM injection, `open_link` validation).
8. Token leakage paths (WS `?token=`, on-disk modes, `browser.storage.local`).
9. Worktree poisoning / git config hook execution.
10. CORS / origin policy bypass.
11. Supervisor restart bombs.

## Critical (block v2)

None.

## Important (should fix before v2)

**Bearer-token comparison is not constant-time** —
`crates/libre-cr-review/src/server/auth.rs:63` (`presented == expected`) and
`auth.rs:71` (`t == expected`) — A local attacker observing latency on the
loopback interface (e.g., a colocated low-privilege process) could in principle
do timing-recovery on the 256-bit token. The practical exploitability on
localhost is low (loopback jitter dominates per-byte branch timing), but token
checks are a textbook case for `subtle::ConstantTimeEq`. Fix: compare via a
constant-time routine.

**Pairing-code redemption has no rate limit and no failure delay** —
`crates/libre-cr-review/src/pairing.rs:34` redeems on a `HashMap::remove` with
no per-IP counter. The 6-byte (48-bit) code is comfortable against blind
brute-force in the 5-minute TTL (`pairing.rs:20`), but a co-resident process
attempting ~10⁵ redemptions/sec over loopback will *not* be slowed. Fix: cap
to e.g. 5 wrong attempts per minute per IP, with a 200 ms artificial delay on
each failure, and clear all pending codes after N failures. Also: increase the
code from 6 to 8 random bytes (still trivially typeable when hex-encoded).

**Git refs and SHAs pass to the `git` CLI without `--` separation or a
leading-dash check** —
`crates/libre-cr-code/src/repo/worktree.rs:69-71` (`fetch origin <ref_name>`),
`crates/libre-cr-code/src/git/diff.rs:30` (`format!("{from_ref}..{to_ref}")`),
`crates/libre-cr-code/src/git/show.rs:20,45` (`sha`), and
`crates/libre-cr-code/src/git/blame.rs:32` (`ref_name`) all forward
attacker-influenceable strings to `git`. A ref starting with `--` (e.g.,
`--upload-pack=...`) would be interpreted as a flag. Git rejects refs starting
with `-`, but the *caller* doesn't validate — refs flow in from
`pr_data.head_ref` scraped by the extension and from LLM tool calls, both of
which the threat model treats as untrusted. **Exploit sketch:** an attacker who
controls extension input or who has prompt-injected the LLM submits
`head_ref = "--upload-pack=/tmp/evil.sh"`; `git fetch` interprets it as a flag
and spawns the script. Fix: reject any ref/sha starting with `-`, and insert
`--` between subcommand arguments and refs (e.g., `git diff --unified=3 --
<from>..<to>`). This is defense-in-depth on top of git's own checks but the
right hardening for a tool whose ref values originate outside the trust
boundary.

## Suggestions (nice to have)

- **`presented`/`expected` length-prefix in auth check.** A length mismatch
  shortcuts string equality; preferable to compare on a known-length canonical
  representation.
- **Config file mode is not enforced.** Spec
  (`specs/08-distribution.md:253`) says the daemon "refuses to start with a
  config file with mode wider than 0644." Not implemented in
  `crates/libre-cr-review/src/config.rs:178-186` — `Config::load` reads the file
  unconditionally. The token file at `cli.rs:160-166` is created 0600 only on
  first write; an existing token file with wider mode is left as-is.
  Recommend: stat both files at startup, refuse on `(mode & 0o077) != 0`.
- **`isSafeUrl` operator-precedence trap, technically benign but fragile.**
  `extension/utils/presentation/handlers.ts:224` —
  `parsed.protocol === "https:" || parsed.protocol === "http:" && ...`
  parses as `https || (http && 127.0.0.1)` thanks to `&&` binding tighter.
  That happens to be the desired behavior here (allow any https; allow http
  only on loopback), but the intent is opaque and one reordering away from
  allowing `http://anywhere`. Wrap with explicit parens.
- **`open_link` allows arbitrary HTTPS URLs.** Spec
  (`specs/09-presentation-tools.md:262`) calls out "https or known-safe
  origin"; the implementation is just "any https". The LLM is influenced by PR
  content (prompt injection vector). Worth at least logging the URL pre-open
  and considering a confirm-on-non-GitHub-origin policy.
- **`SpawnedClient` `stderr(Stdio::null())`** (`code_daemon/transport.rs:83`)
  swallows the code daemon's diagnostics. Not a vulnerability, but it deletes
  the audit trail you'd want when a tool result looked tampered with.
- **`http_api` smoke test exercises pairing via the in-process `PairingStore`
  handle**, but in production *no route or CLI command publishes a code to the
  running daemon's `PairingStore`.** `crates/libre-cr-cli/src/pair.rs:17` and
  `crates/libre-cr-review/src/cli.rs:73-75` both build a *local* throwaway
  store and print a code that the daemon will reject on `/v1/pair`. This is a
  functionality gap (pairing doesn't actually work), but it has a *security*
  upside: until the gap is closed, `/v1/pair` can never successfully return
  the token, so the lack of rate-limiting is moot. When the gap is closed,
  ship the rate-limiting at the same time.

## Confirmed good

- **`safe_join` rejects absolute paths, `..` escapes, and symlinks that point
  outside the repo** (`crates/libre-cr-code/src/util.rs:13-53`), with tests
  covering each. Combined with `canonicalize` post-check, this is correct
  defense against the standard path-traversal class.
- **Token, install-key, MCP socket all written 0600 on Unix** (`cli.rs:164`,
  `crypto.rs:43`, `mcp/server.rs:52`). XDG dirs are tightened to 0700 by the
  wrapper (`paths.rs:81`).
- **AES-GCM with a random 32-byte install key and 12-byte random nonce per
  encrypt** (`storage/crypto.rs:56-69`). Standard library use, no nonce reuse.
  Spec was honest that "machine-bound" is a per-install random key; the
  implementation matches.
- **Bearer token is 32 random bytes from `OsRng`, URL-safe-base64**
  (`cli.rs:153-156`). Plenty of entropy.
- **`Origin` enforcement after pairing** (`auth.rs:95-113`). Mismatched origin
  is 403; missing-origin (curl, MCP) is allowed only because the token already
  authenticated. CORS layer adds preflight rejection for browser callers.
- **WS `?token=` fallback is gated on the `Upgrade: websocket` header**
  (`auth.rs:69`) — query-param token is *only* accepted for actual upgrade
  requests. No HTTP request log captures query strings (axum doesn't log by
  default, no `tower-http::trace` enabled).
- **WS query token lifetime is short**: the URL exists during the upgrade and
  in the browser's network panel; it does not appear in
  `browser.storage.local` or in any persistent log
  (`extension/utils/daemon/ws.ts:145-148`).
- **`annotate_line` uses `textContent`, not `innerHTML`**
  (`extension/utils/presentation/handlers.ts:120,125`). Color/severity are
  enum-mapped to class names, never interpolated as CSS or HTML. This was an
  explicit spec requirement; it's satisfied.
- **`git worktree add --detach FETCH_HEAD`** is used
  (`crates/libre-cr-code/src/repo/worktree.rs:91-97`). `--detach` means HEAD
  isn't moved on a branch; combined with the fact that we never `checkout` and
  never `run hooks`, the worktree-poisoning surface is small. Hooks live in
  the *upstream* repo's `.git/hooks` (not fetched), not in worktree files;
  fetched files are inert on disk until a tool reads them.
- **Supervisor restart budget is 5 in 60s**
  (`crates/libre-cr-cli/src/supervisor.rs:74-82`), and `RestartBudget` on the
  review-daemon side enforces a similar window for the code-daemon child
  (`crates/libre-cr-review/src/code_daemon/budget.rs`). A flapping crash loop
  cannot exhaust CPU/forking indefinitely.
- **Subprocess invocation uses `Command::new` + per-arg `.arg()`** everywhere
  observed — no `sh -c`, no shell expansion, no string-concatenation of
  arguments. The git-CLI calls are not susceptible to shell injection per se
  (only to git's own flag parsing — see Important finding above).

## Out-of-scope acknowledgments

- **No daemon-side sandbox.** Per `REVIEW.md` § cross-cutting and
  `specs/04-review-daemon.md` § Configuration, the user's machine is in the
  trust boundary; a compromised daemon can read the user's repos and write to
  their disk. Correctly out of scope.
- **API-key encryption is not strong against a determined local attacker.**
  `REVIEW.md` § `04-review-daemon.md` explicitly accepts this; the install key
  sits next to the ciphertext in the same 0600 directory. The mitigation is
  the file mode and "your account is the trust boundary." Confirmed
  correctly noted, not silently dropped.
- **LLM prompt injection from PR content.** `specs/09-presentation-tools.md`
  § Threat Model Notes acknowledges open_link, scroll, and annotate as the
  LLM-driven surfaces. The mitigations are the URL validator + DOM
  `textContent` injection + spec-level prompt guidance. The residual risk
  (the LLM is talked into highlighting decoy lines or opening a phishing
  https URL) is accepted; this is the same risk any LLM agent operating on
  user-supplied input has.
- **GitHub OAuth / posting reviews** is Phase 9 and out of v2. Clipboard
  hand-off avoids the credential-storage problem entirely. Correctly deferred.
- **`browser.storage.local` is not a hardened secrets store.** Spec
  (`specs/05-browser-extension.md:217`) says "stored encrypted with the
  extension's own obfuscation." Obfuscation, not encryption; correctly
  acknowledged in spec.

## Coverage notes

Thoroughly read: `server/auth.rs`, `server/routes.rs`, `server/ws.rs`,
`pairing.rs`, `storage/crypto.rs`, `code_daemon/{client,transport}.rs`,
`code_daemon::budget`, all of `git/*.rs`, `repo/worktree.rs`,
`tools/fs_tools.rs`, `util.rs::safe_join`, `mcp/server.rs`, `cli.rs` (token
bootstrap), CLI `paths.rs`, `proc.rs`, `supervisor.rs`,
`extension/utils/presentation/handlers.ts`, `extension/utils/daemon/ws.ts`.

Spot-checked: `tools/presentation.rs`, `tools/router.rs`, `tools/internal.rs`,
`provider/*.rs`, `storage/store.rs`, `repo/registry.rs`,
extension manifest / state-storage shape.

Not exercised: dynamic behavior against a real Anthropic/OpenAI endpoint;
behavior under Windows ACLs (token/install-key permission story is Unix-only
in this read); fuzzing of MCP wire frames from a malicious code daemon.
