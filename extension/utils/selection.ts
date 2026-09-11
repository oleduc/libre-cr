// Selection model. Mirrors `libre-cr-common::Selection`.

export type Selection =
  | { kind: "line"; file: string; line: number; text?: string }
  | { kind: "range"; file: string; start_line: number; end_line: number; text?: string }
  | {
      kind: "symbol";
      file: string;
      line: number;
      column: number;
      identifier: string;
      text?: string;
    }
  // A GitHub review thread annotating a diff line. The unit is the thread:
  // the reply is often where the answer lives. `comment_id` is the top
  // comment's id (GitHub's `databaseId`), and `comments` is oldest first.
  | {
      kind: "comment";
      comment_id: string;
      file: string;
      line: number;
      side: "left" | "right";
      comments: { author: string; body: string }[];
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
    case "comment": {
      const who = s.comments[0]?.author ?? "comment";
      const more = s.comments.length - 1;
      return `${s.file}:${s.line} comment by ${who}${more > 0 ? ` +${more}` : ""}`;
    }
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
