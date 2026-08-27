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

const FILE_CONTAINER_SEL = '[data-tagsearch-path], table[aria-label^="Diff for: "]';
const NUM_CELL_SEL = "td.blob-num, td[data-line-number]";
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
  const numCells = row.querySelectorAll<HTMLTableCellElement>(NUM_CELL_SEL);
  for (const cell of Array.from(numCells)) {
    const v = cell.getAttribute("data-line-number") ?? cell.textContent?.trim() ?? "";
    const n = Number(v);
    if (Number.isFinite(n) && n > 0) {
      return { file, line: n, side: sideOf(cell) };
    }
  }
  return null;
}

/** Locate the `<tr>` for a given (file, line) — for highlight overlay. */
export function findRow(
  file: string,
  line: number,
  root: ParentNode = globalThis.document,
): HTMLTableRowElement | null {
  const fileEl = Array.from(root.querySelectorAll<HTMLElement>(FILE_CONTAINER_SEL)).find(
    (el) => filePathOf(el) === file,
  );
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
