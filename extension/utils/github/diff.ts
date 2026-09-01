// Diff utilities — enumerate visible diff lines and hit-test clicks.
//
// Two GitHub DOMs are supported (see `selectors.ts`): classic pages expose
// `td.blob-num[data-line-number]` under a `[data-tagsearch-path]` container;
// the React "changes" UI exposes `td[data-line-number][data-diff-side]` under
// `table[aria-label="Diff for: <path>"]`.

export interface DiffLineRef {
  file: string;
  line: number;
  side?: "L" | "R";
}

export interface ResolvedLine extends DiffLineRef {
  /** The `<tr>` element containing the line — useful for highlight overlay. */
  row: HTMLTableRowElement;
}

/** Per-file diff container, either DOM. */
export const FILE_CONTAINER_SEL = '[data-tagsearch-path], table[aria-label^="Diff for: "]';
/** Line-number cells, either DOM. */
export const NUM_CELL_SEL = "td.blob-num, td[data-line-number]";
/** Code cells, either DOM. */
export const CODE_CELL_SEL = "td.blob-code, td.diff-text-cell";
const DIFF_FOR = "Diff for: ";

/** File path of a diff container matched by `FILE_CONTAINER_SEL`, either DOM. */
export function filePathOf(el: Element): string | null {
  const classic = el.getAttribute("data-tagsearch-path");
  if (classic) return classic;
  const label = el.getAttribute("aria-label") ?? "";
  return label.startsWith(DIFF_FOR) ? label.slice(DIFF_FOR.length) : null;
}

function sideOf(cell: Element): "L" | "R" {
  return cell.classList.contains("blob-num-deletion") || cell.getAttribute("data-diff-side") === "left"
    ? "L"
    : "R";
}

function ancestorMatching<T extends Element>(node: Node | null, selector: string): T | null {
  let n: Node | null = node;
  while (n && n.nodeType !== 1) n = n.parentNode;
  let el = n as Element | null;
  while (el) {
    if (el.matches?.(selector)) return el as T;
    el = el.parentElement;
  }
  return null;
}

/** Walk every visible diff row and yield `{file, line, row}` for each. */
export function enumerateDiffLines(root: ParentNode = globalThis.document): ResolvedLine[] {
  const out: ResolvedLine[] = [];
  const files = root.querySelectorAll<HTMLElement>(FILE_CONTAINER_SEL);
  for (const fileEl of Array.from(files)) {
    const file = filePathOf(fileEl);
    if (!file) continue;
    const rows = fileEl.querySelectorAll<HTMLTableRowElement>("tr");
    for (const row of Array.from(rows)) {
      // The relevant cell — last numeric one in the row is the "right side" /
      // unified line. We accept either.
      const numCells = row.querySelectorAll<HTMLTableCellElement>(NUM_CELL_SEL);
      let line: number | null = null;
      let side: "L" | "R" | undefined;
      for (const cell of Array.from(numCells)) {
        const v = cell.getAttribute("data-line-number") ?? cell.textContent?.trim() ?? "";
        const n = Number(v);
        if (Number.isFinite(n) && n > 0) {
          line = n;
          side = sideOf(cell);
        }
      }
      if (line !== null) {
        out.push({ file, line, row, side });
      }
    }
  }
  return out;
}

/**
 * Identify the file and line for a given DOM node — useful when a user clicks
 * something inside a diff row.
 */
export function hitTestLine(node: Node | null): DiffLineRef | null {
  if (!node) return null;
  const row = ancestorMatching<HTMLTableRowElement>(node, "tr");
  if (!row) return null;
  const fileEl = ancestorMatching<HTMLElement>(row, FILE_CONTAINER_SEL);
  const file = fileEl ? filePathOf(fileEl) : null;
  if (!file) return null;
  // React UI: the clicked cell itself carries the line number and side (code
  // cells included), so honour the side the user actually clicked. Classic
  // DOM code cells don't, so fall back to the row's first numbered cell.
  const own = ancestorMatching<HTMLTableCellElement>(node, "td[data-line-number]");
  const cells = own && own.closest("tr") === row ? [own] : [];
  cells.push(...Array.from(row.querySelectorAll<HTMLTableCellElement>(NUM_CELL_SEL)));
  for (const cell of cells) {
    const v = cell.getAttribute("data-line-number") ?? cell.textContent?.trim() ?? "";
    const n = Number(v);
    if (Number.isFinite(n) && n > 0) {
      return { file, line: n, side: sideOf(cell) };
    }
  }
  return null;
}

/** The rendered diff container for `file`, if GitHub has mounted it. */
export function fileContainer(file: string, root: ParentNode = globalThis.document): HTMLElement | null {
  return (
    Array.from(root.querySelectorAll<HTMLElement>(FILE_CONTAINER_SEL)).find(
      (el) => filePathOf(el) === file,
    ) ?? null
  );
}

const BIDI_MARKS = /[\u200e\u200f]/g;

/**
 * Make GitHub render `file`'s diff. The React "changes" UI virtualizes: files
 * away from the viewport are placeholder regions (`[id^="diff-"][role="region"]`
 * whose heading is the path) with no rows, so effects targeting them fail with
 * `file_not_in_view`. Scrolling the placeholder (or clicking the file-tree
 * link) into view makes GitHub mount the table; we then wait for it.
 * Resolves `true` when rows exist. Classic DOM: always already rendered.
 */
export async function ensureFileRendered(
  file: string,
  root: Document = globalThis.document,
): Promise<boolean> {
  if (fileContainer(file, root)) return true;
  const textOf = (el: Element | null) => el?.textContent?.replace(BIDI_MARKS, "").trim() ?? "";
  const region = Array.from(
    root.querySelectorAll<HTMLElement>('[id^="diff-"][role="region"]'),
  ).find((r) => {
    const heading = r.getAttribute("aria-labelledby");
    return !!heading && textOf(root.getElementById(heading)) === file;
  });
  const treeLink = region
    ? null
    : Array.from(root.querySelectorAll<HTMLAnchorElement>('a[href^="#diff-"]')).find(
        (a) => textOf(a) === file,
      );
  if (!region && !treeLink) return false;
  // ponytail: this moves the reviewer's viewport to the file; acceptable for a
  // companion that is about to point at it anyway.
  if (region) region.scrollIntoView({ block: "start" });
  else treeLink!.click();
  const deadline = Date.now() + 4000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 120));
    if (fileContainer(file, root)) return true;
  }
  return false;
}

/** The code text of `file`'s lines `start..=end` as rendered in the diff,
 *  without our caption chips; undefined when nothing is rendered. Capped so a
 *  huge drag never bloats the question payload. */
export function textOfLines(
  file: string,
  start: number,
  end: number,
  root: ParentNode = globalThis.document,
): string | undefined {
  const out: string[] = [];
  for (let l = start; l <= Math.min(end, start + 120); l++) {
    const row = findRow(file, l, root);
    const cell = row?.querySelector<HTMLElement>(CODE_CELL_SEL);
    if (!cell) continue;
    const clone = cell.cloneNode(true) as HTMLElement;
    clone.querySelectorAll(".libre-cr-label").forEach((c) => c.remove());
    out.push(clone.textContent ?? "");
  }
  if (out.length === 0) return undefined;
  const text = out.join("\n");
  return text.length > 4000 ? `${text.slice(0, 4000)}…` : text;
}

/**
 * Scroll `el` into view and make sure it stayed there. A `smooth` scroll is
 * cancelled by any other scroll or layout shift, and GitHub's virtualized diff
 * shifts layout as rows mount — so scroll instantly, then re-check the
 * element's position a few times and re-scroll if it moved.
 */
export async function scrollIntoViewSettled(
  el: Element,
  block: ScrollLogicalPosition = "center",
): Promise<void> {
  if (typeof el.scrollIntoView !== "function") return; // jsdom
  const win = el.ownerDocument.defaultView;
  for (let attempt = 0; attempt < 4; attempt++) {
    el.scrollIntoView({ behavior: "auto", block });
    await new Promise((r) => setTimeout(r, 150));
    const rect = el.getBoundingClientRect();
    const vh = win?.innerHeight ?? 0;
    if (!vh || (rect.top >= 0 && rect.bottom <= vh)) return;
  }
}

/** Locate the `<tr>` for a given (file, line) — for highlight overlay. */
export function findRow(
  file: string,
  line: number,
  root: ParentNode = globalThis.document,
): HTMLTableRowElement | null {
  const fileEl = fileContainer(file, root);
  if (!fileEl) return null;
  const cells = Array.from(
    fileEl.querySelectorAll<HTMLTableCellElement>(`td[data-line-number="${line}"]`),
  ).filter((c) => c.matches(NUM_CELL_SEL));
  // Prefer the right/new side when both sides carry the number (context rows).
  const cell = cells.find((c) => sideOf(c) === "R") ?? cells[0];
  return cell?.closest("tr") ?? null;
}

/** Regex-pick the identifier under a `(line, column)` for symbol selection. */
export function pickIdentifier(text: string, column: number): string | null {
  if (column < 0 || column >= text.length) return null;
  const re = /[A-Za-z_$][\w$]*/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const start = m.index;
    const end = m.index + m[0].length;
    if (column >= start && column <= end) return m[0];
    if (start > column) break;
  }
  return null;
}
