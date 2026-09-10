# Grounding and Context

## Purpose

Two rules decide whether an answer is worth reading: it has to be *about the
code that is actually there*, and it has to arrive at all. This document is the
contract for both — what evidence reaches the model, how much of it, what
happens when there is too much, and what the reviewer is told when something
was left out.

It exists because both rules failed in the field, in ways no unit test had
reason to catch. Neither failure was a bug in a single function; both were
consequences of *what the model was handed*. That makes them a contract
concern rather than an implementation detail, which is why they are specified
here rather than left to `04-review-daemon.md` § Agent Loop.

| Field failure | What the reviewer saw | Rule it produced |
|---|---|---|
| Fabricated identifiers | The answer described `_is_ambiguous` and `AmbiguousStorageError`, neither of which exists anywhere in the repository. The turn had read only `retry.py`, yet described `single_use_store.py`'s internals. | [Evidence on the wire](#evidence-on-the-wire), [Grounding rules](#grounding-rules) |
| Systematic non-answers | After seven turns on one PR, every further question failed. Nothing was logged and no turn row was written, so the only evidence was an error frame in the browser. | [The context budget](#the-context-budget), [Reporting what was cut](#reporting-what-was-cut) |

## Grounding rules

The system prompt carries these as instructions. They are prompt-level rather
than mechanical because the model is the only component that can decide
whether a claim needs evidence — but each one exists because its absence
produced a wrong answer in the field.

1. **Read before describing.** Before naming or describing a function, class
   or file, read it (`get_pr_diff`, `read_file`, `grep`). Never invent an
   identifier; if it has not been read, say so instead of guessing.
2. **Fresh tool results outrank history.** When a tool result disagrees with
   something said earlier in the conversation, the tool result wins — re-derive
   from it rather than repeating the earlier claim. (An earlier answer once
   described a "CDK rollback" that was never in the PR; once the stale claim
   was in the transcript, later turns kept repeating it.)
3. **Cite only what is currently visible.** Every identifier and `file:line`
   stated must appear verbatim in a tool result the model can see *in this
   request*. Tool output from older turns is not visible (see
   [Conversation context](#conversation-context)); anything first seen there
   must be re-read before it is cited.
4. **Walkthroughs highlight what they describe.** When asked to walk through,
   point out or show part of a PR, each described part is highlighted with its
   own `label` and `detail` — see `09-presentation-tools.md`.

Rule 3 is the one that fails silently: the model has no way to distinguish
"I read this" from "I read a summary of this five turns ago" unless the
context makes the difference explicit. The replay stubs below exist to make it
explicit.

## Evidence on the wire

Grounding instructions are worthless if the evidence needed to follow them
never arrives. Three contracts guarantee it.

### The selection carries its own text

A `Selection` is not only coordinates. Every variant — `line`, `range`,
`symbol` — carries an optional `text` field holding the selected code as
rendered in the diff:

```jsonc
{ "kind": "line", "file": "src/oauth/single_use_store.py", "line": 38,
  "text": "_CONDITIONAL_CHECK_FAILED = \"ConditionalCheckFailedException\"" }
```

The field is optional (`#[serde(default)]`) so an older client still pairs, but
the extension populates it on every selection path: direct cell clicks,
Cmd/Ctrl-click symbol picks, and GitHub's own hash-based line and range
gestures. On a split or replacement row the text comes from the *clicked
side*, so a right-side selection never quotes the deleted line.

`build_user_message` quotes it back to the model in a fenced block, capped at
2,000 characters. Without this the model knew only "line 38" and had to count
lines in an unnumbered blob to find it — which is exactly what it got wrong.

### A review comment arrives with its anchor

A `Selection` of `kind: "comment"` carries a GitHub review thread — the
comments themselves, plus the `file`, `line` and diff `side` the thread
annotates. The anchor is the load-bearing half: it lets the model read the code
the concern is about instead of reasoning from the concern alone, which is the
same principle as quoting the selected line rather than naming it.

`side` is carried because a comment on a removed line has an OLD-side number;
resolving it against the new file would read the wrong line — the identical
failure mode as quoting the wrong side of a replacement row.

*Specified, not built* — see `05-browser-extension.md` § Review-comment
selection.

### Reads are line-numbered

`read_file` returns content with each line prefixed by its 1-based number in a
fixed five-column format (`   38 | …`). The reviewer's "line 38" and the
model's line 38 are then the same line by construction rather than by
counting. See `03-code-daemon.md` § File and structural reads.

### The PR diff is the PR's diff

`get_pr_diff` is computed by the router, not taken from scraped page data: it
runs `git_diff` with `merge_base: true` against `origin/<base>...HEAD` — a
three-dot diff. A two-dot diff includes commits that landed on the base branch
since the PR forked, which is how unrelated files (a CDK stack the PR never
touched) appeared in an answer as if they were part of the change. Optional
`paths` narrows it, and narrowing is strongly encouraged: see
[the context budget](#the-context-budget).

### Worktree management is not the model's job

The PR head is already checked out before the first question. The
worktree-management tools — `clone_repo`, `discover_repo`, `scan_for_repos`,
`prepare_worktree`, `list_worktrees`, `remove_worktree` — are **not offered to
the model** and are refused if called anyway. The system prompt states the
checkout path and base branch and that every code tool already operates there.

Left visible, they got used: with an empty diff and no stated checkout path, a
turn ran `discover_repo` → `scan_for_repos` → `clone_repo` against a guessed
URL, failed on a private repository, and degraded to "I don't have access to
the diff."

## Conversation context

A follow-up question is the common case, and the model needs the evidence its
previous answer rested on. But tool output is the largest thing in a session by
orders of magnitude, so replaying all of it is what caused the second field
failure. The policy is therefore graduated.

For every prior turn that succeeded, in order:

| Tier | What is replayed | When |
|---|---|---|
| Prose | The question and the answer text | Always, for every prior turn within `max_history_messages` |
| Full fidelity | Prose **plus** every tool result verbatim, capped | The most recent `replay_full_turns` turns, plus any turn the reviewer has left expanded |
| Stub | Prose **plus** one line naming the tools used | Every older turn |

The stub is load-bearing, not decoration:

```text
[Tools used this turn: read_file src/x.py, grep single_use_store — outputs no
longer in context; re-read before citing specifics.]
```

Without it the model cannot tell a turn whose evidence it still holds from one
whose evidence is gone, and grounding rule 3 becomes unenforceable from the
model's side.

**Budgeting is by messages, not turns.** `max_history_messages` counts
provider messages, and an ordinary turn contributes two (the question and the
answer). A limit of 30 therefore replays about 14 turns, not 30 — an earlier
implementation counted turns and could send 58 messages against a limit of 30.

### Expanded turns ride along

The panel collapses older exchanges when a new question is asked, so a turn the
reviewer has *deliberately left expanded* is a statement of interest. `AskInit`
carries their daemon ids:

```jsonc
{ "question": "…", "context_turn_ids": ["t_9fa3…", "t_1b77…"] }
```

Those turns replay at full fidelity in addition to the recency floor, capped at
`replay_max_full_turns` with the oldest demoted to stubs first. Ids are matched
only against the session's own turns, so an id from another session is inert.
Restored turns carry their daemon id from history; live turns learn theirs from
the `done` frame.

This is why the gesture is worth honouring: re-expanding an old exchange before
asking about it is what a reviewer does anyway, so the intent signal is free.

## The context budget

Every model has a finite context window, and a review session grows without
bound. The failure mode is not gradual degradation — it is a hard wall, and
past it *every* question fails identically.

Measured on one PR, against a 262,144-token window:

| Quantity | Measured |
|---|---|
| One `get_pr_diff` without `paths` | **589,499 chars** (~168k tokens) |
| That turn's cumulative input across its rounds | **760,326 tokens** |
| Session prose floor after 7 turns | ~33k chars (~9k tokens) |
| Replayed tool output, worst case | ~57k tokens |

So a single tool result could occupy well over half the window, the agent loop
re-sends the whole message array on every round, and the replay floor rises as
the conversation grows. Nothing bounded any of it.

Six caps bound it now. All are configured in `[limits]` and edited in the
daemon's own configuration UI (see `04-review-daemon.md` § Configuration UI):

| Cap | Default | Bounds |
|---|---|---|
| `max_tool_result_chars` | 20,000 | One live tool result |
| `max_turn_tool_chars` | 120,000 | All live tool output for one turn, across every round |
| `replay_full_turns` | 2 | Recent turns replayed with their evidence |
| `replay_max_full_turns` | 5 | Full-fidelity turns per ask, expanded ones included |
| `replay_result_chars` | 20,000 | One replayed tool result |
| `replay_turn_chars` | 40,000 | All replayed tool output for one turn |

`max_tool_turns` and `max_history_messages` bound the same request from the
other two directions and are configured alongside them.

The per-turn budget is not redundant with the per-result cap: the per-result
cap alone still permits `max_tool_turns` × that cap of tool output in a single
request.

**The caps are character counts, not token counts.** A deliberate proxy: exact
tokenization is per-model and would put a tokenizer in the hot path for a
bound that only needs to be approximately right. Roughly 3.5 characters per
token held for the JSON-shaped tool output measured above; a reviewer changing
models should treat the defaults as scaled to a ~262k window.

### Exceeding a cap never fails the turn

This is the part that matters. A cap is a bound on what the model is handed,
not a precondition for answering:

1. The result is **truncated to the allowance**, never dropped.
2. The model is told, in the result body, how to get the rest:
   `…[truncated: 589499 → 20000 chars. Narrow the request to see more: `paths`
   on get_pr_diff, `start_line`/`end_line` on read_file.]` This is the part
   that actually resolves the overflow — the model's *next* call is narrower —
   rather than merely surviving it.
3. A **2,000-character floor** applies even when a turn's whole budget is
   spent, with a note saying so. The model always receives a readable head, so
   the loop can always make progress; it is never handed nothing. The floor is
   not configurable, and it bounds its own overrun at `max_tool_turns` × 2,000.
4. **The store keeps the whole result.** The cap governs what the model can
   hold, not what the export may show. `tool_traces` rows are complete.

Raising a cap past what the model can hold restores the original failure — with
a log line this time (see below).

## Reporting what was cut

A shortened answer that looks complete is worse than a visible gap, so
truncation is reported on both sides.

**To the model:** the note inside the truncated result, above.

**To the reviewer:** the `tool_result` frame carries the original size:

```jsonc
{ "type": "tool_result", "call_id": "c1", "result_preview": { … },
  "truncated_from": 589499 }
```

Absent when nothing was cut. The panel renders two things from it — a
per-trace marker in the thinking trace (`get_pr_diff ⚠ truncated from 589,499
chars`, so it is clear *which* call was shortened) and a per-turn notice under
the answer naming the count, the largest original size, and where to raise the
caps.

**A failed turn leaves evidence.** When a turn fails for any reason it is
logged at error level with the session id, and persisted as a turn with status
`error`. Previously a failure produced no log line and no row: the second field
failure above had to be reconstructed from stored trace sizes and the model's
published context window, which is not a diagnosis path anyone should repeat.

## Failure modes

| Condition | Behaviour |
|---|---|
| One tool result over `max_tool_result_chars` | Truncated with a narrowing note; turn completes |
| Turn's whole tool budget spent | Every further result cut to the 2,000-char floor with a budget note; turn completes |
| Replayed turn over its replay caps | Truncated with `re-read to cite the rest`; older turns already stubbed |
| More expanded turns than `replay_max_full_turns` | Oldest demoted to stubs; newest keep their evidence |
| `context_turn_ids` naming a foreign turn | Ignored — ids are matched against the session's own turns |
| Request still over the model's window | Provider error; error frame to the panel, error log line, `error` turn row |
| Cap set out of range via the API | `400` with the reason; stored config unchanged |

## What this does not do

- **No token counting.** Caps are character-based by choice (see above).
- **No summarization or compaction.** Older tool output is stubbed, not
  condensed. Compacting evidence would mean the model citing a summary as if it
  were the source, which is the failure this document exists to prevent.
- **No automatic cap tuning.** The daemon does not read the model's context
  window and derive defaults. The reviewer sets them; the defaults assume a
  ~262k-token window.
- **No mid-turn context eviction.** Once a turn is running, its message array
  only grows. The caps keep that growth bounded; they do not reclaim.
- **No guarantee of completeness.** A truncated result means the model answered
  from less than it asked for. The contract is that this is *visible*, not that
  it never happens.

## Provenance

Every rule here is traceable to a manual-testing round; the diagnosis for each
is recorded in `CHANGELOG-TESTING.md` under the entries named below.

| Rule | Ledger entry |
|---|---|
| Selection carries text; numbered reads | "The model explained the wrong line" (PR #459 round) |
| Three-dot PR diff | "`get_pr_diff` reported changes that aren't in the PR" |
| Fresh results outrank history | "The phantom CDK rollback came back — from history" |
| Worktree tools hidden | "The model went repo-hunting and called `clone_repo` itself" |
| History replay tiers | "History replay carries recent tool results" |
| Expanded turns as context | "Expanded turns ride along as context" |
| Caps, truncation, reporting | "A long conversation outgrew the model's context, and the failure left no trace" |
