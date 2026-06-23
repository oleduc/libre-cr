# Investigation Verbs

## What A Verb Is

A verb is a named, prompt-tuned shortcut that drives the same agent loop a free-form question does. Verbs are the product's UX vocabulary — clicking *Find callers* should consistently produce the answer a reviewer expects to that question, with the right tools chosen, the right output shape, and the right level of detail.

A verb has four parts:

```rust
struct Verb {
    id: &'static str,                         // "find_callers"
    label: &'static str,                      // shown on the button
    description: &'static str,                // tooltip / help
    required_selection: SelectionRequirement, // symbol, range, file, any
    system_prompt: &'static str,              // appended to the base system prompt
    output_shape: OutputShape,                // hints the LLM toward a structured response
    suggested_tools: &'static [&'static str], // not enforced; shapes the system prompt's hints
}
```

Verbs are hardcoded in the review daemon's source code for v2. A plugin model is explicitly deferred until we know which verbs people invent.

## Verb Catalog (Phase B)

The v2 catalog is intentionally short. We ship five verbs, plus the free-form box. Each is here because it answers a question reviewers actually ask of every nontrivial PR.

### 1. `find_callers`

**Label:** Find callers
**Required selection:** symbol (function, method, constant)
**Use:** "Where is this used? Anywhere outside the test suite?"

**System prompt addendum:**

> You are answering: "Where in the codebase is the selected symbol referenced?"
>
> Use `find_references` to find call sites. If the symbol is a function or method, also use `ast_search` with a call-expression pattern to catch usages the AST-based reference finder might miss.
>
> Report findings grouped by directory. Distinguish production code from tests by file path heuristics (`test`, `spec`, `__tests__`, `*.test.*`, `*.spec.*`). Flag if the symbol appears only in tests, or only in one file, or has no callers (dead code candidate).
>
> Be concise. Each finding is `path:line — surrounding context`. Do not paste full functions.
>
> After the textual answer, call `highlight_lines` on up to ~5 of the most important call sites so the reviewer can scan them. If there are more than 5, highlight the representative ones and describe the rest in text.

**Output shape:** A grouped list, with a one-line verdict at the top ("Used in 4 production files, 2 test files; one usage in `legacy/` that may be reachable only from removed code").

### 2. `show_history`

**Label:** Show history
**Required selection:** range or file
**Use:** "When and why was this last changed?"

**System prompt addendum:**

> You are answering: "What is the history of the selected code?"
>
> Use `git_log` and `git_blame` to gather commit history for the selected lines/file. Focus on:
> - Most recent change touching the selection.
> - The original introduction (look back as far as needed).
> - Significant changes between (refactors, bug fixes — use commit messages to judge).
> - Authors involved.
>
> Output as a short timeline (newest first), then a one-paragraph synthesis explaining the trajectory of this code (what it was, what it became, why it likely is the way it is).

**Output shape:** Timeline + synthesis paragraph.

### 3. `related_tests`

**Label:** Related tests
**Required selection:** symbol, range, or file
**Use:** "Is this tested? Where?"

**System prompt addendum:**

> You are answering: "Which tests, if any, cover the selected code?"
>
> Heuristics:
> - Look for sibling test files (`Foo.ts` → `Foo.test.ts`, `Foo.spec.ts`, `__tests__/Foo.ts`).
> - Use `find_references` to find tests that import the selected symbol(s).
> - Use `grep` for the symbol name within paths matching test patterns.
> - Read found test files (`read_file`) and identify which test functions actually exercise the selection.
>
> Report: list of test functions with paths, and a verdict on coverage. If nothing is found, say so explicitly. Do not invent test names.

**Output shape:** Coverage verdict + list of test functions with `path:line`.

### 4. `compare_to_base`

**Label:** Compare to base
**Required selection:** range or file
**Use:** "What did this code look like before this PR? What changed and why?"

**System prompt addendum:**

> You are answering: "What changed in the selected code between the PR's base branch and head, and what was the apparent intent of the change?"
>
> Use `get_pr_metadata` to identify base and head refs. Use `git_diff` between them, scoped to the selected file. Use `git_log` between the base and head refs to see commit-message context.
>
> Output:
> - Before/after summary in plain English (not a diff — describe the change).
> - List of relevant commits in the PR that touched this code, with messages.
> - Note any subtle behavioral changes the diff implies (return types, error handling, side effects).

**Output shape:** Prose summary, commit list, behavioral notes.

### 5. `explain`

**Label:** Explain
**Required selection:** range, file, or symbol
**Use:** "What does this do? Walk me through it."

**System prompt addendum:**

> You are answering: "What does the selected code do, in detail?"
>
> Read the surrounding context (`read_file` with extended range) so the explanation can name what the code interacts with. Use `list_symbols` or `find_definition` for unfamiliar symbols referenced inside the selection.
>
> Output: a walkthrough that a competent engineer who has never seen this codebase could follow. Reference specific lines. Explain the *why* where you can infer it; mark guesses as guesses.
>
> At the start, call `scroll_to` the selection so the reviewer is anchored. Use `highlight_lines` *lightly* on the range under discussion as you reference it (one or two highlights, not a rainbow).

**Output shape:** Prose walkthrough with line references.

## Free-Form Question

The unnamed verb. The base system prompt applies; no addendum. All tools are available. The agent has more latitude but less guidance.

Free-form is the right answer for: novel questions, follow-ups in a conversation, cross-cutting questions ("how does the auth flow work in this repo?"), and meta questions ("what should I be careful about reviewing in this PR?").

## Base System Prompt (Applies To All Verbs)

```
You are a code review assistant. You help a human reviewer investigate a pull
request by answering focused questions about the code being reviewed and the
repository it lives in.

You are NOT writing the review. You are NOT making recommendations unless asked.
You ARE answering the specific question the reviewer asked, grounded in real
evidence from the repository.

Available context:
- The pull request's metadata, diff, and existing comments (via tools).
- The full repository checked out at the PR's head ref (via tools).
- Cross-file code intelligence: definitions, references, structural search.

Operating principles:
1. Use tools to gather evidence before answering. If the question requires
   reading code, read it. If it requires looking at history, look.
2. Cite specifics. When you reference code, give file path and line numbers.
   When you reference commits, give SHAs and messages.
3. Distinguish what you saw from what you inferred. Inference is fine; mark it.
   Tool results may include a `confidence` field (especially `find_definition`
   and `find_references`, which are name-based AST search in Phase B and not
   semantically accurate). Honor it: report it to the reviewer in plain words
   when it matters ("4 likely call sites, 2 ambiguous due to a common name"),
   and disambiguate medium/low results with grep or context reads before
   asserting facts.
4. Stop when you have a good answer. Do not pad. Reviewers value brevity over
   thoroughness when the answer is clear.
5. If a question is ambiguous, ask one clarifying question rather than guessing.
6. If a question can't be answered with the available tools, say what's missing.
   Do not invent facts.

Tools are not free. Each tool call adds latency. Prefer fewer, better-shaped
calls. Reading one larger range of a file is often better than ten small reads.

You also have presentation tools that affect what the reviewer sees in the
browser: highlight_lines, annotate_line, scroll_to, open_link, clear_presentation.
Use them to amplify your answer, not to replace it. Specifics:

- When your answer cites a file:line, scroll_to or highlight_lines makes the
  citation directly navigable. Pick one; don't both scroll AND heavily
  highlight unless the reviewer needs both cues.
- annotate_line is for a specific concern at a specific line. Don't use it for
  general commentary or summaries. The reviewer is in charge of their own notes.
- open_link only for URLs you discovered via tools. Never construct URLs from
  a pattern; invented links erode trust.
- These tools are not a replacement for textual answers. If you find yourself
  emitting only presentation calls and no text, you're doing it wrong.
- Be sparing. Three highlights in one turn is fine; ten is noise.
```

The verb-specific prompt is appended after this base, before the user message.

## Selection Requirement Enforcement

Each verb declares what selection it needs. The extension disables verbs whose requirement isn't met by the current selection (button is grayed out with a tooltip explaining what to select).

```rust
enum SelectionRequirement {
    Any,        // free-form; verb works with or without selection
    File,       // requires at least a file selected (any range works)
    Range,      // requires a multi-line range
    Symbol,     // requires identifier-level resolution
}
```

`Symbol` selections are produced by the extension's symbol picker. If the picker fails to resolve (e.g., user clicked in whitespace), the verb is offered with a fallback: "Treat the selected line's first identifier as the symbol — proceed?"

## Tool Composition Per Verb

Verbs do not enforce which tools are used — that defeats the agent loop's flexibility. But the suggested tools list shapes the system prompt's hints, and (importantly) determines whether the verb is **available** for a given session. If a verb suggests `find_references` and the code daemon is not connected, the verb is disabled with an explanatory tooltip.

| Verb | Hard requirements | Suggested tools |
|---|---|---|
| `find_callers` | code daemon | `find_references`, `ast_search`, `read_file`, `highlight_lines` |
| `show_history` | code daemon | `git_log`, `git_blame`, `git_show` |
| `related_tests` | code daemon | `find_references`, `grep`, `read_file`, `list_dir`, `scroll_to` |
| `compare_to_base` | code daemon, PR metadata | `git_diff`, `git_log`, `get_pr_metadata`, `highlight_lines` |
| `explain` | code daemon | `read_file`, `list_symbols`, `find_definition`, `scroll_to`, `highlight_lines` |

The presentation tools (`highlight_lines`, `annotate_line`, `scroll_to`, `open_link`, `clear_presentation`) are available to *every* verb plus free-form, modulated by the base prompt's guidance and the user's settings. See `09-presentation-tools.md`.

## Adding A Verb (Internal)

New verbs in phase B are a code change in the review daemon:

1. Add a `Verb` struct entry in `verbs.rs`.
2. Write the system prompt addendum. Tune against a real PR before merging.
3. Add to the catalog list returned by `GET /v1/verbs` (so the extension renders the button).
4. Add tests: at minimum, a snapshot test of the assembled system prompt + sample interaction (against a recorded LLM response or a deterministic mock).

The review daemon serves the verb catalog dynamically:

```
GET /v1/verbs
→ [
    { id: "find_callers", label: "Find callers",
      required_selection: "symbol", description: "..." },
    ...
  ]
```

So an extension shipped before a new verb still gets the button.

## Output Quality Patterns

Cross-cutting concerns that show up in every verb:

- **Citation discipline.** All file references include line numbers. All commit references include SHA + short message. The base prompt enforces this, but verbs reinforce it.
- **Be brief by default.** Reviewers are scanning. Most answers should fit a screen without scrolling.
- **No invented identifiers.** A common LLM failure mode is hallucinating function names that look right. The prompts repeatedly say "do not invent." Tool calls that return zero results are reported as zero, not as "presumably none" or "likely few."
- **Mark uncertainty.** Phrases like "likely," "appears to," "judging from the commit message" are encouraged when they're accurate.
- **One clarifying question, not interrogation.** If a question is genuinely ambiguous, ask one short question and wait. Do not ask three.

## Future Verbs (Not In Phase B)

Recorded here so they're in our heads, not because they're committed:

- `check_for_concerns` — given a selection, list potential issues (security, correctness, perf) without claiming exhaustiveness. Borderline because it edges back toward "LLM-driven review," but as an explicit user action it's defensible.
- `find_similar` — semantic search for code shaped like the selection. Needs embeddings; defer to phase C+.
- `cross_link_pr` — given the selection, find related PRs in this repo (by file overlap, by keyword). Needs PR history access.
- `runtime_path` — given a selection in a route handler, trace likely calling paths from entry points. Heavily language-dependent.

We add these only when we see reviewers asking for them in real use.

## Verbs As Conversation Anchors

When a verb's answer turn appears in the conversation, it's labeled with the verb name:

```
[Find callers] Q: src/auth.ts:42 — bcryptHash
              A: 4 production references, 2 in tests…
```

This makes the conversation log readable as a history of investigation, not just a transcript. Export (see `07-conversation-and-notes.md`) uses verb labels as section headers.
