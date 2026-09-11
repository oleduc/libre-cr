// Hook into GitHub's own line selection instead of owning the gesture.
//
// Clicking a line number sets the URL hash to `#diff-<digest><side><line>`
// and shift-click to `#diff-<digest><side><a>-<side><b>` (side R = new, L =
// old). The digest is SHA-256 of the file path (verified live), so the hash
// decodes with no scraping: hash → path via the rendered diff containers.

import type { Selection } from "../selection";
import { THREAD_SEL, selectionFromThread } from "./comments";
import { FILE_CONTAINER_SEL, filePathOf, textOfLines } from "./diff";

const DIFF_HASH = /^#?diff-([0-9a-f]{64})([RL])(\d+)(?:-[RL](\d+))?$/;

const digestCache = new Map<string, string>();

export async function digestOfPath(path: string): Promise<string> {
  const hit = digestCache.get(path);
  if (hit) return hit;
  const buf = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(path));
  const hex = [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
  digestCache.set(path, hex);
  return hex;
}

/** Decode a GitHub diff hash into a Selection using the currently rendered
 *  diff files (the clicked file is always rendered). Old-side (L) lines keep
 *  their numbers — good enough to anchor a question. */
export async function selectionFromDiffHash(
  hash: string,
  root: ParentNode = globalThis.document,
): Promise<Selection | null> {
  const m = DIFF_HASH.exec(hash);
  if (!m) return null;
  const [, digest, sideRaw, aRaw, bRaw] = m;
  // Quote the side the reviewer selected: a replacement row renders a left
  // (deleted) and a right (added) cell for the same coordinates.
  const side = sideRaw === "L" ? ("left" as const) : ("right" as const);
  const files = new Set(
    Array.from(root.querySelectorAll<HTMLElement>(FILE_CONTAINER_SEL))
      .map((el) => filePathOf(el))
      .filter((p): p is string => !!p),
  );
  for (const file of files) {
    if ((await digestOfPath(file)) !== digest) continue;
    const a = Number(aRaw);
    const b = bRaw ? Number(bRaw) : a;
    if (!Number.isFinite(a) || a <= 0) return null;
    const [lo, hi] = [Math.min(a, b), Math.max(a, b)];
    if (b !== a) {
      return {
        kind: "range",
        file,
        start_line: lo,
        end_line: hi,
        text: textOfLines(file, lo, hi, root, side),
      };
    }
    return { kind: "line", file, line: a, text: textOfLines(file, a, a, root, side) };
  }
  return null;
}

/** Follow GitHub's own selection: parse the current hash now, on every
 *  hashchange, and shortly after any click — GitHub sets the hash through
 *  history.pushState, which fires no hashchange event. Returns a cleanup
 *  function. */
export function watchGithubLineSelection(
  onSelect: (sel: Selection) => void,
  win: Window = window,
): () => void {
  let last = "";
  let generation = 0;
  const apply = () => {
    const hash = win.location.hash;
    if (hash === last) return;
    last = hash;
    // The decode is async (SHA-256 digests); two quick selections can
    // resolve out of order, and a stale result would restore the old one.
    const g = ++generation;
    void selectionFromDiffHash(hash, win.document).then((sel) => {
      if (sel && g === generation) onSelect(sel);
    });
  };
  const afterClick = () => {
    win.setTimeout(apply, 150);
  };
  win.addEventListener("hashchange", apply);
  win.document.addEventListener("click", afterClick, true);
  if (win.location.hash) apply();
  return () => {
    win.removeEventListener("hashchange", apply);
    win.document.removeEventListener("click", afterClick, true);
  };
}

/**
 * Hover affordance for review threads: reveal an "Ask about this" control on
 * the hovered thread; clicking it selects the thread.
 *
 * A modifier-click on the thread itself was rejected — a comment body is full
 * of links, `Reply` and `Resolve`, so hijacking clicks there is materially
 * riskier than in a diff cell.
 *
 * One button is reused for every thread and lives on `documentElement`,
 * outside GitHub's React tree: nothing to re-inject when React re-renders a
 * thread, and no injected node inside a subtree React owns. It is positioned
 * `fixed` from the thread's rect, so it needs no positioned ancestor — and it
 * hides on scroll rather than tracking it, since the pointer has to come back
 * to the thread anyway.
 */
export function installCommentAffordance(
  onSelect: (sel: Selection) => void,
  doc: Document = document,
): () => void {
  let btn: HTMLButtonElement | null = null;
  let thread: Element | null = null;
  /** The selection this thread yields, computed when it is hovered. The
   *  control is only shown when there is one — a file-level thread has no
   *  diff row, so it produces nothing, and a button that does nothing on
   *  click is worse than no button. */
  let pending: Selection | null = null;

  const hide = () => {
    thread = null;
    pending = null;
    if (btn) btn.hidden = true;
  };

  const button = (): HTMLButtonElement => {
    if (btn) return btn;
    const b = doc.createElement("button");
    b.type = "button";
    b.textContent = "Ask about this";
    // Tagged like presentation effects: attributes survive React re-renders,
    // `className` does not.
    b.setAttribute("data-libre-cr-tag", "ask");
    b.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      if (!thread) return;
      // Recompute: a reply may have landed since the hover.
      const sel = selectionFromThread(thread) ?? pending;
      hide();
      if (sel) onSelect(sel);
    });
    doc.documentElement.appendChild(b);
    btn = b;
    return b;
  };

  const over = (ev: Event) => {
    const target = ev.target as Node | null;
    const el = target instanceof Element ? target : target?.parentElement;
    const hovered = el?.closest(THREAD_SEL);
    if (!hovered) return;
    if (hovered === thread) return;
    const sel = selectionFromThread(hovered);
    if (!sel) {
      hide();
      return;
    }
    thread = hovered;
    pending = sel;
    const b = button();
    const rect = hovered.getBoundingClientRect();
    b.style.top = `${Math.max(0, rect.top + 6)}px`;
    b.style.left = `${Math.max(0, rect.right - 118)}px`;
    b.hidden = false;
  };

  const out = (ev: Event) => {
    const to = (ev as MouseEvent).relatedTarget;
    if (to instanceof Element && (to === btn || to.closest(THREAD_SEL) === thread)) return;
    hide();
  };

  doc.addEventListener("mouseover", over, true);
  doc.addEventListener("mouseout", out, true);
  doc.addEventListener("scroll", hide, { capture: true, passive: true });
  return () => {
    doc.removeEventListener("mouseover", over, true);
    doc.removeEventListener("mouseout", out, true);
    doc.removeEventListener("scroll", hide, true);
    btn?.remove();
    btn = null;
  };
}
