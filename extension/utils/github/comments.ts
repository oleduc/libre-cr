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
