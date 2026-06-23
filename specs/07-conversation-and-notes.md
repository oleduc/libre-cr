# Conversation and Notes

## The Mental Model

Code review is an *investigation*. A reviewer reads, asks, reads more, sometimes finds something worth flagging, sometimes confirms something is fine, eventually arrives at a verdict. The valuable artifact at the end is not the transcript of the investigation — it's the conclusions the reviewer reached.

This product captures both, but treats them differently:

- **Conversation turns** are the investigation. Useful while reviewing. Mostly disposable afterward.
- **Notes** are the conclusions. Explicit. Meant for the final review.

The export step assembles notes into a review draft. Conversation turns are *available* in the export but excluded by default; they're context, not output.

## What Lives In The Review Daemon

Per `04-review-daemon.md`, the SQLite schema has three relevant tables: `sessions`, `turns`, `tool_traces`. Recap:

- A **session** = one PR (`pr_url` is unique).
- A **turn** is a single interaction: a question with its answer, or a standalone note. Ordinal per session.
- **Tool traces** record the agent's tool calls per turn — useful for transparency and debugging, optional in export.

Notes are turns with `kind = "note"`. They have `user_content` and optional `selection`, `severity`. They never have an `answer` or tool calls.

## How A Reviewer Creates Notes

Two paths:

### 1. Explicit user note

The Q&A panel's input box has two buttons: "Add note" and "Ask ▶".

- **Ask ▶** does what it says.
- **Add note** posts the input as a note. No LLM call.

A note can have an attached selection (the user's current selection at the time) and a severity. Severity is selected from a small picker that appears when you click "Add note":

```
[ + Add note ]
   ┌──────────────────────────────────┐
   │ ◯ Info                            │
   │ ● Suggestion                     │
   │ ◯ Warning                        │
   │ ◯ Critical                       │
   └──────────────────────────────────┘
   ┌──────────────────────────────────┐
   │ The bcrypt cost of 12 is fine    │
   │ but we should match the rest of  │
   │ the codebase which uses 10.      │
   └──────────────────────────────────┘
                       [Cancel] [Add ▶]
```

Notes are visually distinct in the conversation timeline (no thinking trace, gray-and-yellow border, severity icon).

### 2. Agent-flagged note via `add_note` tool

If a verb's system prompt instructs the agent to flag issues (we may add such a verb later), the agent can call the internal `add_note` tool. That creates a turn with `kind = "note"`, `user_content = <agent's note>`, `severity = <as specified>`, and a small marker that it was agent-created. The reviewer sees it inline, can edit or delete it, and it participates in export.

Phase B does not include a verb that auto-flags issues, but the mechanism is there so verbs can use it sparingly when the answer is "yes, there is an issue and it's specific enough to record."

## Editing Notes

Notes are editable. Q&A turns are not.

- Click a note → edit-in-place input.
- PATCH `/v1/sessions/:id/notes/:note_id` updates `user_content` / `severity`.
- DELETE removes the note from history (export will not include it).

Q&A turns are immutable history. If a reviewer disagrees with an answer, they can ask a follow-up; they cannot retroactively edit the original turn. This keeps the conversation log honest.

## Pinning A Turn As A Note

Sometimes a Q&A answer itself is something the reviewer wants to surface in the final review. The conversation panel has a "Save as note" action on every assistant turn:

```
Q: Where is bcryptHash used outside tests?
A: 1 reference: src/auth/legacy.ts:88. The path is guarded by
   IS_LEGACY_USERS=true which is set only on a deprecated worker job.
   [Save as note ▼]
```

Clicking "Save as note":

1. Prompts for a severity (defaults to `info`).
2. Optionally edit the text.
3. POSTs `/v1/sessions/:id/notes` with the (edited) content and a back-reference to the source turn id.

This creates a new note turn. The original Q&A turn is unchanged. Severity attaches to the note, not the answer.

## Conversation View (Recap from extension spec)

```
┌──────────────────────────────────────────────────┐
│ PR #123 — Conversation                            │
├──────────────────────────────────────────────────┤
│ [Find callers] Q: bcryptHash usage?               │
│                A: 4 prod, 2 test references…      │
│                [▾ thinking · save as note · ⋯]    │
├──────────────────────────────────────────────────┤
│ Note  ⚠ Warning  src/auth/legacy.ts:88           │
│      Legacy path still calls md5. Verify          │
│      IS_LEGACY_USERS is removed before merge.     │
│      [Edit · Delete]                              │
├──────────────────────────────────────────────────┤
│ Q: any other md5 usages anywhere?                 │
│ A: One in vendor/legacy-shim.ts. …                │
└──────────────────────────────────────────────────┘
```

Notes get distinctive styling (severity icon, persistent header). Q&A turns get a thinking-trace toggle and a "save as note" action.

## Export

The export endpoint assembles notes into a Markdown draft. Conversation turns are optionally included as context.

### Default export (notes only)

```markdown
# Review: PR #123 — feat: bcrypt migration

## ⚠ Warning

- `src/auth/legacy.ts:88` — Legacy path still calls md5. Verify
  IS_LEGACY_USERS is removed before merge.

## Suggestions

- `src/auth.ts:42` — The bcrypt cost of 12 is fine but the rest of the
  codebase uses 10. Consider matching for consistency.

## Info

- Coverage check: tests in `auth.test.ts` exercise both happy path and
  legacy fallback.

---
Reviewed with libre-cr.
```

Group order: Critical → Warning → Suggestion → Info. Within each group, notes are sorted by `created_at` (chronological).

### Verbose export (notes + selected turns)

```markdown
# Review: PR #123 — feat: bcrypt migration

## ⚠ Warning

- `src/auth/legacy.ts:88` — Legacy path still calls md5.

  <details>
  <summary>Investigation</summary>
  Asked: "Where is bcryptHash used outside tests?"
  Found: 1 reference in src/auth/legacy.ts:88, guarded by IS_LEGACY_USERS.
  </details>

…
```

The reviewer picks the format in the export modal:

```
Export review draft
  ◯ Notes only (clean)
  ◉ Notes + investigation context
  ◯ Full transcript (for personal reference)

Format
  ◉ Markdown (clipboard)
  ◯ GitHub review (structured)

[Cancel] [Export ▶]
```

### Format: GitHub review (structured)

For a future direct-post path (deferred from v2), the daemon returns:

```json
{
  "body": "<overall review summary>",
  "event": "COMMENT" | "REQUEST_CHANGES" | "APPROVE",
  "comments": [
    { "path": "src/auth.ts", "line": 42, "body": "..." },
    { "path": "src/auth/legacy.ts", "line": 88, "body": "..." }
  ]
}
```

The `event` defaults to:
- `REQUEST_CHANGES` if any note is `critical`.
- `COMMENT` otherwise.
- The reviewer can override.

In v2 with no OAuth, this structured form is returned to the extension which:
- Copies the body to clipboard (for the main review textarea).
- Shows a list of inline-comment drafts the user can paste one by one. We do not auto-fill GitHub's inline comment composer; that's a brittle DOM dance.

A future v2.x can add OAuth-backed posting that uses this structure directly.

## Conversation Lifetime

- Sessions persist indefinitely by default. They cost ~KB per session in SQLite; no urgency to delete.
- A configurable idle eviction cleans sessions with `last_active_at` older than `session_idle_evict_days` (default 90). User can change in config.
- Conversation turns are evicted with their session — never independently.
- Notes are evicted with their session. The export draft is the artifact meant to live in GitHub.

The popup's "recent sessions" list shows up to 50, regardless of age.

## Reopening A Past Session

When the reviewer opens the same PR again later (e.g., the author pushed updates):

1. Extension calls `POST /v1/sessions` with fresh scraped PR data.
2. Daemon finds the existing session, updates `pr_data` with the new scrape.
3. Daemon checks: did the PR head ref change? If yes, calls code daemon to refresh the worktree.
4. Conversation history is loaded. New questions append to the existing thread.

The reviewer can see their previous investigation. Notes they wrote previously are still there. If the diff changed substantially, they can decide whether prior notes are still valid (and edit or delete them).

We do not auto-invalidate notes on diff change. The reviewer is in charge. We display a banner: "PR diff changed since your last review · review your notes."

## Cross-Session Search (Power User)

The conversation FTS index supports:

```
GET /v1/search?q=<query>&limit=20
→ [
    { session_id, pr_url, turn_id, snippet, score },
    …
  ]
```

Used by the popup's search box. Lets the reviewer find "where did I previously discuss this same problem?" across all PRs.

This is also what the `session_history_search` internal tool uses, letting the agent cite past turns within the same session.

## What Notes Are Not

- **Not GitHub comments.** Notes live in the daemon's DB. They appear in the export draft, which the user then turns into GitHub comments. The daemon is not posting.
- **Not synced with anyone.** Single-user, single-machine.
- **Not versioned (yet).** Editing a note overwrites it. Edit history is not retained.
- **Not used as training data.** Local only.

## Privacy Of Notes

Notes can contain anything the reviewer typed. Some notes will quote PR code. None of this leaves the machine unless:

- The reviewer triggers export and pastes into GitHub.
- The reviewer asks a follow-up question; the daemon may include past notes as context, and the configured LLM provider then sees them in the prompt.

The second point is worth flagging in the daemon's config UI: "Notes you save are visible to the LLM in subsequent questions about the same PR." Reviewers who type sensitive context into notes should know.
