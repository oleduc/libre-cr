// Review comments, extracted from the page's embedded JSON payload.
//
// Why not the DOM: comment threads are virtualized in GitHub's React
// "changes" UI. On a PR with 29 threads, exactly one was mounted — so a DOM
// scrape would report almost nothing and look like "no discussion".
//
// The payload carries every thread regardless of what is rendered, and it is
// already inside the user's authenticated session, which is what makes this
// work on a private repo. The REST API would be authoritative and versioned,
// but the daemon holds no GitHub token (OAuth posting is Phase 9), so that is
// a later option, not this one.
//
// Shape, read off a live PR (2026-09-10) — none of it is a documented
// contract, so every access is defensive:
//
//   payload.pullRequestsChangesRoute
//     .markers.threads["<threadId>"]        → { isResolved, resolvedBy,
//                                               commentsData.comments[] }
//     .markers.threadsPageInfo.hasNextPage  → GitHub itself paginated
//     .diffSummaries[]                      → { path, markersMap }
//       .markersMap["R188"]                 → { threads: [{ id, start: "R169" }] }
//
// The `R188` / `L42` key is the side and the thread's end line; `start` is the
// same encoding for its first line. That is the identical R/L convention the
// diff URL hash uses (see `gh-selection.ts`).

import type { Selection } from "../selection";
import { FILE_CONTAINER_SEL, filePathOf } from "./diff";

/** One review comment, flattened with its thread's anchor. */
export interface ScrapedComment {
  /** GitHub's thread id; comments in one thread share it. */
  thread_id: string;
  /** `databaseId` — the same id the REST API uses for this comment. */
  comment_id: number;
  author: string;
  body: string;
  /**
   * What the thread is attached to:
   * - `line` — a diff line, so `file`, `line` and `side` are all present;
   * - `file` — the whole file (GitHub's file-level review comments): `file`
   *   only;
   * - `none` — the payload gives no anchor at all. Measured at 17 of 29
   *   threads on one PR, one of them unresolved, so these are not droppable:
   *   the body is still the concern, the model just has to locate it.
   */
  anchor: "line" | "file" | "none";
  file?: string;
  /** The thread's last annotated line. */
  line?: number;
  /** Present when the thread annotates a range. */
  start_line?: number;
  /** `right` = the new/added side, `left` = the old/removed side. */
  side?: "left" | "right";
  /** A resolved thread's concern was already dealt with — the model needs to
   *  know, or it re-raises settled points. */
  resolved: boolean;
  resolved_by?: string;
  created_at?: string;
}

export interface ScrapedComments {
  comments: ScrapedComment[];
  /** Comments found in the payload, before any cap. */
  total: number;
  /** True when this list is a subset — our caps, or GitHub's own pagination. */
  truncated: boolean;
}

/** Max comments carried in `pr_data`. It is stored in the session row and
 *  re-sent on every session init, so it cannot be unbounded. */
export const MAX_COMMENTS = 100;
/** Max chars per body, for the same reason. The daemon caps the tool result
 *  again on the way to the model (see `10-grounding-and-context.md`). */
export const MAX_BODY_CHARS = 1_200;

/** Decode a `markersMap` key or `start` value: `"R188"` → right, line 188. */
function decodeAnchor(raw: string): { side: "left" | "right"; line: number } | null {
  const m = /^([RL])(\d+)$/.exec(raw);
  if (!m) return null;
  return { side: m[1] === "L" ? "left" : "right", line: Number(m[2]) };
}

function clip(body: string): string {
  if (body.length <= MAX_BODY_CHARS) return body;
  return `${body.slice(0, MAX_BODY_CHARS)}…[truncated: ${body.length} chars]`;
}

type Anchor = Pick<ScrapedComment, "anchor" | "file" | "line" | "start_line" | "side">;

/**
 * Anchors by thread id, built from every file's `markersMap`.
 *
 * Keys are `R188` / `L42` (side + line) or the literal `FILE` for a
 * file-level comment. Thread ids arrive as numbers here and as object keys in
 * `markers.threads`, so they are compared as strings — a `Set` of the raw
 * values silently matches nothing.
 */
function anchorsByThread(route: Record<string, unknown>) {
  const out = new Map<string, Anchor>();
  const summaries = Array.isArray(route.diffSummaries) ? route.diffSummaries : [];
  for (const summary of summaries as Record<string, unknown>[]) {
    const file = typeof summary?.path === "string" ? summary.path : null;
    const map = summary?.markersMap;
    if (!file || !map || typeof map !== "object") continue;
    for (const [key, entry] of Object.entries(map as Record<string, unknown>)) {
      const end = decodeAnchor(key);
      const threads = (entry as Record<string, unknown>)?.threads;
      if (!Array.isArray(threads)) continue;
      for (const t of threads as Record<string, unknown>[]) {
        if (t?.id === undefined || t?.id === null) continue;
        if (!end) {
          // `FILE`, or a key shape we do not know: the file is still known.
          out.set(String(t.id), { anchor: "file", file });
          continue;
        }
        const start = typeof t.start === "string" ? decodeAnchor(t.start) : null;
        out.set(String(t.id), {
          anchor: "line",
          file,
          line: end.line,
          side: end.side,
          ...(start && start.line !== end.line ? { start_line: start.line } : {}),
        });
      }
    }
  }
  return out;
}

/**
 * Pull review comments out of the embedded payload.
 *
 * Returns `null` when the payload is absent or unrecognisable — the caller
 * must treat that as "unknown", never as "no comments", so a shape change on
 * GitHub's side cannot silently look like a PR with no discussion.
 */
export function extractComments(payloadJson: string | null | undefined): ScrapedComments | null {
  if (!payloadJson) return null;
  let root: Record<string, unknown>;
  try {
    root = JSON.parse(payloadJson) as Record<string, unknown>;
  } catch {
    return null;
  }
  const route = (root.payload as Record<string, unknown> | undefined)?.pullRequestsChangesRoute as
    | Record<string, unknown>
    | undefined;
  const markers = route?.markers as Record<string, unknown> | undefined;
  const threads = markers?.threads;
  if (!route || !threads || typeof threads !== "object") return null;

  const anchors = anchorsByThread(route);
  const pageInfo = markers?.threadsPageInfo as Record<string, unknown> | undefined;
  const out: ScrapedComment[] = [];
  let total = 0;

  for (const [threadId, thread] of Object.entries(threads as Record<string, unknown>)) {
    const t = thread as Record<string, unknown>;
    // A thread with no anchor is still emitted: `markersMap` covers only the
    // lines that survive in the current diff, and an unresolved concern on a
    // line since rewritten is exactly the kind of thing worth asking about.
    const anchor: Anchor = anchors.get(threadId) ?? { anchor: "none" };
    const comments = (t?.commentsData as Record<string, unknown> | undefined)?.comments;
    if (!Array.isArray(comments)) continue;
    total += comments.length;
    for (const c of comments as Record<string, unknown>[]) {
      if (out.length >= MAX_COMMENTS) break;
      const author = (c?.author as Record<string, unknown> | undefined)?.login;
      const body = typeof c?.body === "string" ? c.body : "";
      if (!body) continue;
      out.push({
        thread_id: threadId,
        comment_id: typeof c.databaseId === "number" ? c.databaseId : 0,
        author: typeof author === "string" ? author : "unknown",
        body: clip(body),
        ...anchor,
        resolved: t?.isResolved === true,
        ...(typeof t?.resolvedBy === "string" ? { resolved_by: t.resolvedBy } : {}),
        ...(typeof c?.createdAt === "string" ? { created_at: c.createdAt } : {}),
      });
    }
  }

  return {
    comments: out,
    total,
    truncated: out.length < total || pageInfo?.hasNextPage === true,
  };
}

// ---------------------------------------------------------------------------
// Selection: reading the one thread the reviewer is pointing at.
//
// This half reads the DOM, unlike the payload parse above, and deliberately:
// selection needs the element under the cursor, which is by definition
// mounted. Virtualization only breaks the *whole-PR* list.

// Two DOMs again, as everywhere else in this file (see `selectors.ts`): the
// React "changes" UI renders a thread as `[data-testid="review-thread"]` with
// `id="r<databaseId>"` per comment, and the classic Conversation tab renders
// it as `.js-resolvable-timeline-thread-container` with
// `id="discussion_r<databaseId>"` per comment and its own diff hunk. Both were
// read off live PRs (2026-09-10). CSS-module class names in the React UI
// (`ReviewThread-module__…`) are build-hashed — never selectors.
export const THREAD_SEL =
  '[data-testid="review-thread"], .js-resolvable-timeline-thread-container';
const COMMENT_ID = /^(?:r|discussion_r)(\d+)$/;
/** Same cap the other selection variants use for captured text. */
const MAX_SELECTION_BODY_CHARS = 4_000;

function authorOf(el: Element): string {
  const byText = el.querySelector("a.author")?.textContent?.trim();
  if (byText) return byText;
  const href =
    el.querySelector('[data-testid="avatar-link"]')?.getAttribute("href") ??
    el.querySelector('a[href^="/"]')?.getAttribute("href") ??
    "";
  const login = href.replace(/^\//, "").split(/[/?#]/)[0];
  return login || "unknown";
}

function bodyOf(el: Element): string {
  const text = (el.querySelector(".markdown-body, .comment-body")?.textContent ?? "").trim();
  return text.length > MAX_SELECTION_BODY_CHARS
    ? `${text.slice(0, MAX_SELECTION_BODY_CHARS)}…`
    : text;
}

/**
 * Where the thread points, in the Conversation tab's DOM.
 *
 * There is no enclosing diff table there: the thread carries its own hunk, and
 * the annotated line is its **last** numbered row — the hunk is the context
 * *above* the comment. The path is the header link's text.
 */
function timelineAnchor(thread: Element): { file: string; line: number; side: "left" | "right" } | null {
  const file = thread.querySelector("a.text-mono")?.textContent?.trim();
  const cells = thread.querySelectorAll("td.blob-num[data-line-number]");
  const cell = cells[cells.length - 1];
  const line = Number(cell?.getAttribute("data-line-number"));
  if (!file || !Number.isFinite(line) || line <= 0) return null;
  return { file, line, side: cell?.classList.contains("blob-num-deletion") ? "left" : "right" };
}

/**
 * Turn a hovered review thread into a `Selection`.
 *
 * Returns `null` unless the thread yields both an anchor (file, line, side)
 * and at least one comment body — a selection missing either would send the
 * model a concern it cannot locate, or a location with no concern.
 */
export function selectionFromThread(thread: Element): Selection | null {
  const container = thread.closest(FILE_CONTAINER_SEL);
  // The thread's *own* row carries the annotated line in the React diff UI. An
  // earlier design read the preceding code row and was off by one against the
  // REST API.
  const cell = thread.closest("tr")?.querySelector("td[data-line-number][data-diff-side]");
  const anchor = container
    ? (() => {
        const file = filePathOf(container);
        const line = Number(cell?.getAttribute("data-line-number"));
        if (!file || !Number.isFinite(line) || line <= 0) return null;
        return {
          file,
          line,
          side: cell?.getAttribute("data-diff-side") === "left" ? ("left" as const) : ("right" as const),
        };
      })()
    : timelineAnchor(thread);
  if (!anchor) return null;
  const { file, line, side } = anchor;

  // One element per comment: `id="r<databaseId>"` (React) or
  // `id="discussion_r<databaseId>"` (Conversation). If that shape ever
  // changes, fall back to the bodies alone rather than losing the thread.
  const roots = Array.from(thread.querySelectorAll<HTMLElement>("[id]")).filter((el) =>
    COMMENT_ID.test(el.id),
  );
  const comments = roots.length
    ? roots.map((el) => ({ author: authorOf(el), body: bodyOf(el) }))
    : Array.from(thread.querySelectorAll<HTMLElement>(".markdown-body, .comment-body")).map(
        (el) => ({ author: authorOf(thread), body: bodyOf(el.parentElement ?? el) }),
      );
  const kept = comments.filter((c) => c.body.length > 0);
  if (!kept.length) return null;

  return {
    kind: "comment",
    comment_id: COMMENT_ID.exec(roots[0]?.id ?? "")?.[1] ?? "",
    file,
    line,
    side,
    comments: kept,
  };
}
