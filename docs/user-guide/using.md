# Using Libre CR

This page walks through a review session from opening the PR to pasting the finished review into GitHub.

## Open a PR

Navigate to any GitHub pull request. The content script detects the PR page and mounts a floating Libre CR button near the PR header. Clicking it opens the **Q&A panel** — a draggable floating widget. *[screenshot: Q&A panel over a PR diff]*

Behind the scenes, the extension scrapes the PR (title, branches, description, diff) and posts it to the review daemon, which finds your local checkout of that repo and silently materializes the PR head into a managed worktree. While that fetch runs, the panel shows "Preparing repo…" — answers that need repo access wait for it. Worktrees are detached (you can't accidentally commit into one) and are evicted least-recently-used past a 5 GiB budget.

## Select code

Verbs and questions operate on your current selection, shown as a chip in the panel header (`Selection: src/auth.ts:42–48 · ×`):

- **Line** — click a line number in the diff gutter.
- **Range** — shift-click to extend, or drag across lines.
- **Symbol** — Cmd-click (Ctrl-click on Linux/Windows) an identifier; the extension resolves the token under the cursor.

The selection is sticky until you clear (×) or replace it. Asking with no selection is allowed — it means "ask about this PR in general."

## The five verbs

Verbs are buttons in the panel — prompt-tuned shortcuts that run the same agent loop a typed question does. Buttons gray out when the current selection doesn't satisfy the verb's requirement (the tooltip explains what to select).

| Verb | Needs | Ask this when… |
|---|---|---|
| **Find callers** | symbol | …you want to know where a function/constant is used, and whether only in tests (dead-code check included). |
| **Show history** | range or file | …you want to know when and why this code last changed — a commit timeline plus a synthesis of its trajectory. |
| **Related tests** | symbol, range, or file | …you want to know whether this code is tested and which test functions actually exercise it. |
| **Compare to base** | range or file | …you want a plain-English before/after of what this PR changed here, with the commits and any subtle behavioral shifts. |
| **Explain** | symbol, range, or file | …you want a line-referenced walkthrough of what the selected code does, in context. |

## Free-form questions

The text box at the bottom is the unnamed verb: type anything, **Enter** sends (**Shift-Enter** for a newline). Use it for follow-ups, cross-cutting questions ("how does the auth flow work in this repo?"), or meta questions ("what should I be careful about in this PR?"). Answers stream in token by token; one question runs at a time.

## Thinking trace

Every answer carries a collapsed **thinking trace** — the sequence of tool calls the agent made (file reads, greps, git log, …) with truncated result previews. Click `[▾] thinking trace (N tool calls)` to expand it. Look here when you're skeptical of an answer: it shows exactly what evidence was gathered. Older Q&A turns auto-collapse to one-line summaries; click to re-expand.

## Presentation effects

While answering, the assistant may highlight diff lines, attach a small annotation, scroll the diff to a cited location, or surface a link — only ever as part of answering *your* question. The panel footer counts active effects:

```
2 highlights · 1 annotation · [Clear all]
```

- **Clear all** removes every effect immediately.
- Effects also auto-clear when you ask the next question or close the panel.
- The **mute toggle** (🔊/🔇 in the title bar) disables presentation effects for this session only — answers still arrive as text. The setting persists per session.

## Notes and severities

Conversation turns are the investigation; **notes** are the conclusions you intend to ship in the review.

- Type into the question box and click **Add note** instead of *Ask* — saves the text as a note (no LLM call), with the current selection attached.
- Notes carry a severity: **Info**, **Suggestion**, **Warning**, or **Critical**. Edit a note inline to change text or severity; notes can be deleted. Q&A turns are immutable — disagree with an answer by asking a follow-up.

### Save an answer as a note

Every assistant turn has a **Save as note ▼** action: pick a severity (defaults to info), optionally edit the text, and it becomes a note linked back to the source turn. Use this when an investigation result *is* the finding.

## Export

Click the **⇪** button in the panel title bar to open the export modal:

- **Content**: *Notes only* (clean draft) · *Notes + investigation context* (each note gets a collapsible "Investigation" detail) · *Full transcript* (personal reference).
- **Format**: *Markdown* — rendered draft, copied straight to your clipboard · *GitHub review (structured)* — a body plus per-file/line comment drafts shown for manual pasting.
- A minimum-severity filter lets you drop info/suggestion noise from the draft.

The Markdown draft groups notes Critical → Warning → Suggestion → Info. Paste it into GitHub's review composer ("Files changed" → *Review changes*). Libre CR never posts to GitHub itself — the structured format's inline comments are listed for you to paste one by one; direct OAuth posting is planned.

## The popup

The toolbar icon's popup shows:

- **Daemon status** — connected / not paired / unreachable.
- **Recent sessions** — your last-touched PRs (top 5); click to jump back to a PR you reviewed yesterday.
- **Cross-session search** — full-text search over every past conversation and note ("where did I discuss this connection-pool bug before?"). Matches show highlighted snippets and link to the PR.
- **Configure daemon** (opens the config UI) and **Options/Pair extension** buttons.

## When the PR changes under you

Reopening a PR resumes its session: prior Q&A and notes are all there. If the author pushed new commits since your last visit, the daemon refreshes the worktree to the new head and the panel shows a banner:

> PR diff changed since your last review · review your notes

Notes are **not** auto-invalidated — you decide which still apply (edit or delete the rest). Dismissing the banner is remembered per head commit, so it reappears only if the PR moves again.

## Selector warnings

GitHub's markup changes over time. If the scraper hits an element it no longer fully recognizes, the panel shows a soft "Selector warnings" banner listing what looked off. Answers usually still work (the repo-side tools don't depend on the DOM); treat the banner as a heads-up that diff-anchored features (highlights, selections) may be degraded — and worth reporting.
