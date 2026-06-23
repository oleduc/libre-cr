// Selection model. Mirrors `libre-cr-common::Selection`.

export type Selection =
  | { kind: "line"; file: string; line: number }
  | { kind: "range"; file: string; start_line: number; end_line: number }
  | {
      kind: "symbol";
      file: string;
      line: number;
      column: number;
      identifier: string;
    };

export function selectionFile(s: Selection): string {
  return s.file;
}

export function selectionLabel(s: Selection): string {
  switch (s.kind) {
    case "line":
      return `${s.file}:${s.line}`;
    case "range":
      return `${s.file}:${s.start_line}-${s.end_line}`;
    case "symbol":
      return `${s.file}:${s.line} ${s.identifier}`;
  }
}

export function selectionSatisfies(
  required: "any" | "file" | "range" | "symbol",
  sel: Selection | null,
): boolean {
  if (required === "any") return true;
  if (!sel) return false;
  if (required === "file") return true;
  if (required === "range") return sel.kind === "range" || sel.kind === "symbol";
  if (required === "symbol") return sel.kind === "symbol";
  return false;
}
