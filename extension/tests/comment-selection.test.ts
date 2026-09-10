import { beforeEach, describe, expect, it, vi } from "vitest";

import { selectionFromThread } from "../utils/github/comments";
import { installCommentAffordance } from "../utils/github/gh-selection";
import type { Selection } from "../utils/selection";

// Mirrors the live React "changes" DOM (verified 2026-09-10): the thread lives
// in its own `tr` inside the file's diff table, and *that* row carries the
// annotated line and side. Comment roots are `id="r<databaseId>"`.
function thread(opts: { comments: { login: string; body: string }[]; side?: string }): string {
  const comments = opts.comments
    .map(
      (c, i) => `
      <div id="r${1000 + i}">
        <a data-testid="avatar-link" href="/${c.login}"></a>
        <div class="markdown-body">${c.body}</div>
      </div>`,
    )
    .join("");
  return `
  <table aria-label="Diff for: src/auth.ts"><tbody>
    <tr>
      <td class="blob-num" data-line-number="35">35</td>
      <td class="diff-text-cell" data-diff-side="right" data-line-number="35">const t = 1;</td>
    </tr>
    <tr>
      <td data-line-number="36" data-diff-side="${opts.side ?? "right"}"></td>
      <td><div data-testid="review-thread">${comments}</div></td>
    </tr>
  </tbody></table>`;
}

describe("selectionFromThread", () => {
  it("anchors to the thread's own row and keeps every comment in order", () => {
    document.body.innerHTML = thread({
      comments: [
        { login: "coderabbitai[bot]", body: "This retries forever." },
        { login: "oleduc", body: "Intentional." },
      ],
    });
    const sel = selectionFromThread(document.querySelector('[data-testid="review-thread"]')!);
    expect(sel).toEqual({
      kind: "comment",
      comment_id: "1000",
      file: "src/auth.ts",
      line: 36,
      side: "right",
      comments: [
        { author: "coderabbitai[bot]", body: "This retries forever." },
        { author: "oleduc", body: "Intentional." },
      ],
    });
  });

  it("carries the old side for a comment on a removed line", () => {
    document.body.innerHTML = thread({ comments: [{ login: "r", body: "gone?" }], side: "left" });
    const sel = selectionFromThread(document.querySelector('[data-testid="review-thread"]')!);
    expect(sel).toMatchObject({ side: "left", line: 36 });
  });

  it("falls back to bodies when the comment-id shape changes", () => {
    document.body.innerHTML = thread({ comments: [{ login: "r", body: "still readable" }]}).replace(
      'id="r1000"',
      'id="comment-1000"',
    );
    const sel = selectionFromThread(document.querySelector('[data-testid="review-thread"]')!);
    expect(sel).toMatchObject({ comment_id: "", comments: [{ body: "still readable" }] });
  });

  it("returns null without an anchor or without a body", () => {
    document.body.innerHTML = '<div data-testid="review-thread"><div class="markdown-body">x</div></div>';
    expect(selectionFromThread(document.querySelector('[data-testid="review-thread"]')!)).toBeNull();
    document.body.innerHTML = thread({ comments: [{ login: "r", body: "" }] });
    expect(selectionFromThread(document.querySelector('[data-testid="review-thread"]')!)).toBeNull();
  });
});

describe("installCommentAffordance", () => {
  let onSelect: (s: Selection) => void;
  let seen: Selection[];
  let cleanup: () => void;

  beforeEach(() => {
    seen = [];
    onSelect = (s) => seen.push(s);
    document.body.innerHTML = thread({ comments: [{ login: "r", body: "concern" }] });
    cleanup = installCommentAffordance(onSelect, document);
  });

  const ask = () => document.querySelector<HTMLButtonElement>('button[data-libre-cr-tag="ask"]');

  it("appears on hover, selects the thread, and hides again", () => {
    expect(ask()).toBeNull();
    const el = document.querySelector('[data-testid="review-thread"]')!;
    el.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    const btn = ask()!;
    expect(btn.hidden).toBe(false);
    // It lives outside GitHub's tree, so React re-rendering a thread cannot
    // strip it and nothing needs re-injecting.
    expect(btn.parentElement).toBe(document.documentElement);
    expect(btn.closest('[data-testid="review-thread"]')).toBeNull();

    btn.click();
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatchObject({ kind: "comment", file: "src/auth.ts", line: 36 });
    expect(btn.hidden).toBe(true);
    cleanup();
  });

  it("hides on scroll and is removed on cleanup", () => {
    document
      .querySelector('[data-testid="review-thread"]')!
      .dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    expect(ask()!.hidden).toBe(false);
    document.dispatchEvent(new Event("scroll"));
    expect(ask()!.hidden).toBe(true);
    cleanup();
    expect(ask()).toBeNull();
  });

  it("ignores hovers outside a thread", () => {
    document.querySelector("table")!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    expect(ask()).toBeNull();
    cleanup();
  });

  it("stays hidden on a thread that yields no selection", () => {
    // A file-level review comment renders as a thread with no diff row —
    // observed live, 1 of 8 mounted threads on a Kubernetes PR. Offering a
    // control that cannot select anything is worse than offering none.
    document.body.innerHTML =
      '<div data-testid="review-thread"><div id="r1"><div class="markdown-body">file-level</div></div></div>';
    document
      .querySelector('[data-testid="review-thread"]')!
      .dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    expect(ask()).toBeNull();
    cleanup();
  });
});

describe("selection label", () => {
  it("names the thread's author and reply count", async () => {
    const { selectionLabel } = await import("../utils/selection");
    expect(
      selectionLabel({
        kind: "comment",
        comment_id: "1",
        file: "src/a.ts",
        line: 36,
        side: "right",
        comments: [
          { author: "bot", body: "x" },
          { author: "me", body: "y" },
        ],
      }),
    ).toBe("src/a.ts:36 comment by bot +1");
  });
});
