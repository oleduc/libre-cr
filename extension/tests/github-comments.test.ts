import { describe, expect, it } from "vitest";

import { MAX_BODY_CHARS, extractComments } from "../utils/github/comments";

// Mirrors the shape read off a live PR page (see comments.ts for the map).
// Thread A: two comments (a reply), resolved, right side, single line.
// Thread B: open, left side, annotates a range.
// Thread C: present in `markers` but absent from every `markersMap` — no
//           anchor, so it cannot be pointed at a line.
function payload(over: Record<string, unknown> = {}): string {
  return JSON.stringify({
    payload: {
      pullRequestsChangesRoute: {
        markers: {
          threads: {
            A: {
              isResolved: true,
              resolvedBy: "maintainer",
              commentsData: {
                comments: [
                  {
                    author: { login: "reviewer" },
                    body: "This retries forever.",
                    databaseId: 111,
                    createdAt: "2026-09-01T10:00:00Z",
                  },
                  { author: { login: "author" }, body: "Capped it.", databaseId: 112 },
                ],
              },
            },
            B: {
              isResolved: false,
              commentsData: {
                comments: [{ author: { login: "reviewer" }, body: "Dead code?", databaseId: 222 }],
              },
            },
            C: {
              isResolved: false,
              commentsData: {
                comments: [{ author: { login: "reviewer" }, body: "Orphan.", databaseId: 333 }],
              },
            },
          },
          threadsPageInfo: { hasNextPage: false },
          ...(over.markers as Record<string, unknown>),
        },
        diffSummaries: [
          { path: "src/auth.ts", markersMap: { R188: { threads: [{ id: "A" }] } } },
          {
            path: "src/old.ts",
            markersMap: { L42: { threads: [{ id: "B", start: "L40" }] } },
          },
        ],
        ...(over.route as Record<string, unknown>),
      },
    },
  });
}

describe("extractComments", () => {
  it("anchors comments to file, line, and side, and keeps replies", () => {
    const out = extractComments(payload())!;
    const a = out.comments.filter((c) => c.thread_id === "A");
    expect(a.map((c) => c.comment_id)).toEqual([111, 112]);
    expect(a[0]).toMatchObject({
      anchor: "line",
      author: "reviewer",
      file: "src/auth.ts",
      line: 188,
      side: "right",
      resolved: true,
      resolved_by: "maintainer",
      created_at: "2026-09-01T10:00:00Z",
    });
    expect(a[0].start_line).toBeUndefined();
    // The reply inherits the thread's anchor and resolved state.
    expect(a[1]).toMatchObject({ file: "src/auth.ts", line: 188, resolved: true });
  });

  it("carries the range start and the left side", () => {
    const out = extractComments(payload())!;
    const b = out.comments.find((c) => c.thread_id === "B")!;
    expect(b).toMatchObject({ file: "src/old.ts", line: 42, start_line: 40, side: "left" });
    expect(b.resolved).toBe(false);
  });

  it("keeps unanchored threads, marked as such", () => {
    const out = extractComments(payload())!;
    const c = out.comments.find((x) => x.thread_id === "C")!;
    // Measured: 17 of 29 threads on a real PR had no anchor, one of them
    // unresolved. Dropping them would hide live concerns.
    expect(c).toMatchObject({ anchor: "none", body: "Orphan." });
    expect(c.file).toBeUndefined();
    expect(c.line).toBeUndefined();
    expect(out.total).toBe(4);
    expect(out.truncated).toBe(false);
  });

  it("keeps file-level threads with the file but no line", () => {
    const raw = JSON.parse(payload());
    raw.payload.pullRequestsChangesRoute.diffSummaries[0].markersMap = {
      FILE: { threads: [{ id: "A" }] },
    };
    const out = extractComments(JSON.stringify(raw))!;
    const a = out.comments.find((c) => c.thread_id === "A")!;
    expect(a).toMatchObject({ anchor: "file", file: "src/auth.ts" });
    expect(a.line).toBeUndefined();
    expect(a.side).toBeUndefined();
  });

  it("reports truncation when GitHub itself paginated the threads", () => {
    const out = extractComments(
      payload({ markers: { threadsPageInfo: { hasNextPage: true } } }),
    )!;
    expect(out.truncated).toBe(true);
  });

  it("clips long bodies", () => {
    const long = "x".repeat(MAX_BODY_CHARS + 500);
    const raw = JSON.parse(payload());
    raw.payload.pullRequestsChangesRoute.markers.threads.A.commentsData.comments[0].body = long;
    const out = extractComments(JSON.stringify(raw))!;
    const c = out.comments[0];
    expect(c.body.length).toBeLessThan(long.length);
    expect(c.body).toContain("truncated");
  });

  it("returns null — not an empty list — when the payload is unusable", () => {
    expect(extractComments(null)).toBeNull();
    expect(extractComments("not json")).toBeNull();
    expect(extractComments(JSON.stringify({ payload: {} }))).toBeNull();
  });

  it("returns an empty list when the PR genuinely has no threads", () => {
    const out = extractComments(
      JSON.stringify({
        payload: { pullRequestsChangesRoute: { markers: { threads: {} }, diffSummaries: [] } },
      }),
    )!;
    expect(out).toMatchObject({ comments: [], total: 0, truncated: false });
  });
});
