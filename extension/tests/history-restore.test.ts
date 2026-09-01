import { describe, expect, it } from "vitest";

import { turnsFromSession } from "../components/ContentApp";
import type { SessionTurnRow } from "../utils/daemon/frames";

describe("history restore", () => {
  it("maps stored turns to panel turns, marking failures and notes", () => {
    const rows: SessionTurnRow[] = [
      { turn_id: "t1", kind: "question", status: "ok", question: "why?",
        answer: "because.", selection: { kind: "line", file: "a.rs", line: 3 } },
      { turn_id: "t2", kind: "question", status: "cancelled", question: "and?" },
      { turn_id: "t3", kind: "note", status: "ok", user_content: "check this", severity: "warning" },
    ];
    const turns = turnsFromSession(rows);
    expect(turns).toEqual([
      { kind: "qa", id: "t1", question: "why?", sel: "a.rs:3", answer: "because.",
        collapsed: true, error: undefined },
      { kind: "qa", id: "t2", question: "and?", sel: undefined, answer: "",
        collapsed: true, error: "turn cancelled — no answer" },
      { kind: "note", id: "t3", noteId: "t3", content: "check this", severity: "warning" },
    ]);
  });
});
