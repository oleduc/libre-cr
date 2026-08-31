// Hook into GitHub's own line selection instead of owning the gesture.
//
// Clicking a line number sets the URL hash to `#diff-<digest><side><line>`
// and shift-click to `#diff-<digest><side><a>-<side><b>` (side R = new, L =
// old). The digest is SHA-256 of the file path (verified live), so the hash
// decodes with no scraping: hash → path via the rendered diff containers.

import type { Selection } from "../selection";
import { FILE_CONTAINER_SEL, filePathOf } from "./diff";

const DIFF_HASH = /^#?diff-([0-9a-f]{64})[RL](\d+)(?:-[RL](\d+))?$/;

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
  const [, digest, aRaw, bRaw] = m;
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
    if (b !== a) {
      return { kind: "range", file, start_line: Math.min(a, b), end_line: Math.max(a, b) };
    }
    return { kind: "line", file, line: a };
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
  const apply = () => {
    const hash = win.location.hash;
    if (hash === last) return;
    last = hash;
    void selectionFromDiffHash(hash, win.document).then((sel) => {
      if (sel) onSelect(sel);
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
