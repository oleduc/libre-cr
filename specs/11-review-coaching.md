# 11 — Review Coaching (optional slice)

> **Status: specified, not built.** Written before the implementation. A
> vertical slice — daemon routes, storage, agent work and panel UI — kept in
> one crate so it can be read, disabled, or deleted as a unit.

## What it is

libre-cr helps a reviewer understand a PR. This helps them get better at
reviewing. A reviewer states an intent — *"I want to stop rubber-stamping
migrations"*, *"I want to catch missing tests"* — and each review they finish is
read back against it: what they asked, what they noted, what they never looked
at. Over time the slice keeps the record of that.

It is **not core**. A libre-cr with this compiled out is the product as
specified in `01`–`10`, minus a tab. Everything here is opt-in, off by default,
and local.

## Why it is a slice and not a plugin

The obvious framing is "an addon", which implies a plugin API: module
registration, an event bus, a UI extension point. That API is not built, and
building it *for this* would design it against one imagined consumer.

So the discipline is the reverse: build the slice in-tree, and hold it to one
rule — **it may only reach the daemon through interfaces we would be willing to
publish.** No private store access, no reading another crate's tables, no
patching the agent loop. When a second module exists, the seam is whatever this
one actually used, and extracting it is a rename rather than a redesign.

The interfaces that already qualify: `GET /v1/sessions`, `GET /v1/sessions/:id`
(turns, traces, presentation calls), `POST /v1/sessions/:id/export`, the verb
catalog, and the WS ask protocol. If the slice needs something they do not
give, that gap is the first real evidence about what a module API owes a
module — and it gets recorded here rather than worked around.

## What v1 does, and what it refuses to do

**Does:**

- **Goals.** The reviewer writes an intent in their own words. The slice turns
  it into a small set of checkable questions with the agent's help, which the
  reviewer edits and approves. The approved text is the contract; nothing is
  scored against a goal the reviewer never saw.
- **A read-back, per review.** When a review is exported, the slice asks the
  agent one question against the diff and the session: *given this goal, what
  did this review not look at?* The answer names files, hunks and concerns —
  anchored, like every other answer in this product (`10-grounding-and-context.md`).
- **A record.** Goal, review, read-back, and what the reviewer did with it,
  kept locally so the next read-back can say "this is the third time".

**Refuses to:**

- **Score.** No number, no grade, no progression chart in v1. A model grading
  review quality with no ground truth produces a figure that moves with the
  model, not with the reviewer, and nobody can tell an improvement from noise.
  Coaching that says *"you did not ask about the migration's rollback path"* is
  falsifiable against the diff. A 7.4 is not.
- **Judge on its own opinion where an outcome exists.** Where the world already
  answered — a thread resolved by a code change rather than by "not an issue", a
  concern another reviewer raised on the same PR, a later fix touching lines the
  review never mentioned — the outcome is the signal and the model's job is to
  explain it. Those signals are lagging and noisy, which is an argument for
  showing them sparingly, not for replacing them with something confident.
- **Leave the machine.** The record is a local database. There is no sync, no
  sharing, no export shaped like a leaderboard. A review-quality metric is one
  export away from being a management metric, and the cheapest way not to
  become that is to have nothing to upload.

## Shape

| Piece | Where |
|---|---|
| Storage, goals, read-backs | `crates/libre-cr-coach`, its own SQLite file under the daemon's data dir |
| HTTP surface | `/v1/coach/*`, mounted by the review daemon only when the slice is enabled |
| Agent work | The existing ask protocol, as a verb — the slice sends questions, it does not touch the loop |
| UI | One panel tab, hidden unless `GET /v1/health` reports the slice enabled |
| Config | `[coach] enabled = false` in `review.toml` |

Nothing about the review daemon changes while it is off: no routes, no tab, no
tables, no tools in the model's list, and no cost in the context budget —
which is the test of whether "optional" is true.

## Open questions

Recorded rather than guessed, to be answered by building it:

1. **What the read-back reads.** The session's turns are what the reviewer
   *asked*; the exported review is what they *said*. The gap between them is
   the interesting signal, and neither alone is enough. Whether the slice needs
   the notes' anchors, the presentation calls, or the diff itself is unknown
   until the first read-back is written.
2. **When it runs.** On export is the obvious moment. Whether a reviewer wants
   it *during* a review — a nudge at question three — or only after, is a
   question about how it feels to use, not one to settle on paper.
3. **Cadence.** A reviewer sees 5–20 PRs a week. Any weekly aggregate is mostly
   variance. How much history makes a claim about a trend honest is unknown,
   and until it is, the slice makes no trend claims.
4. **Whether the outcome signals are reachable.** Thread resolution state comes
   with the scraped comments (`04-review-daemon.md` § Internal Tools). Whether
   "a later fix touched lines this review missed" is reachable without a GitHub
   token is not established, and it is the strongest of the three.

## First thing to build

Not the goals, not the storage, not the tab: **one read-back, run by hand
against a real finished review.** It needs no schema and no UI, and it answers
the only question that matters — whether an agent reading a reviewer's own
review against the diff says anything worth hearing. If it does not, the rest
of this document is moot and cheaply abandoned.
