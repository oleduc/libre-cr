// Diff utilities — enumerate visible diff lines and hit-test clicks.
//
// The DOM shape varies between GitHub's "split" and "unified" views, but both
// expose `td.blob-num` cells with `data-line-number` and the file path lives
// on the wrapping `[data-tagsearch-path]` container.

export interface DiffLineRef {
  file: string;
  line: number;
  side?: "L" | "R";
}

export interface ResolvedLine extends DiffLineRef {
  /** The `<tr>` element containing the line — useful for highlight overlay. */
  row: HTMLTableRowElement;
}

const FILE_CONTAINER_SEL = "[data-tagsearch-path]";

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
    const file = fileEl.getAttribute("data-tagsearch-path");
    if (!file) continue;
    const rows = fileEl.querySelectorAll<HTMLTableRowElement>("tr");
    for (const row of Array.from(rows)) {
      // The relevant cell — last numeric one in the row is the "right side" /
      // unified line. We accept either.
      const numCells = row.querySelectorAll<HTMLTableCellElement>("td.blob-num");
      let line: number | null = null;
      let side: "L" | "R" | undefined;
      for (const cell of Array.from(numCells)) {
        const v = cell.getAttribute("data-line-number") ?? cell.textContent?.trim() ?? "";
        const n = Number(v);
        if (Number.isFinite(n) && n > 0) {
          line = n;
          side = cell.classList.contains("blob-num-deletion") ? "L" : "R";
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
  const file = fileEl?.getAttribute("data-tagsearch-path");
  if (!file) return null;
  const numCells = row.querySelectorAll<HTMLTableCellElement>("td.blob-num");
  for (const cell of Array.from(numCells)) {
    const v = cell.getAttribute("data-line-number") ?? cell.textContent?.trim() ?? "";
    const n = Number(v);
    if (Number.isFinite(n) && n > 0) {
      const side = cell.classList.contains("blob-num-deletion") ? "L" : "R";
      return { file, line: n, side };
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
  const fileEl = root.querySelector<HTMLElement>(
    `${FILE_CONTAINER_SEL}[data-tagsearch-path="${cssEscape(file)}"]`,
  );
  if (!fileEl) return null;
  const cell = fileEl.querySelector<HTMLTableCellElement>(
    `td.blob-num[data-line-number="${line}"]`,
  );
  return cell?.closest("tr") ?? null;
}

function cssEscape(s: string): string {
  // Cheap escape good enough for path-like strings; CSS.escape isn't always
  // available in older test envs.
  const ce = (globalThis as unknown as { CSS?: { escape?: (s: string) => string } }).CSS?.escape;
  if (ce) return ce(s);
  return s.replace(/(["'\\\n])/g, "\\$1");
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
