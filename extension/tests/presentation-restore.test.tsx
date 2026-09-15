import { describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { ConversationTurn, type Turn } from "../components/ConversationTurn";
import { createPresentationManager } from "../utils/presentation";
import { turnsFromSession } from "../components/ContentApp";

const FIXTURE = `
<div class="file" data-tagsearch-path="src/a.ts"><table>
  <tr><td class="blob-num" data-line-number="1"></td><td class="blob-code">a</td></tr>
  <tr><td class="blob-num" data-line-number="2"></td><td class="blob-code">b</td></tr>
</table></div>`;

const answered = (): Turn => ({
  kind: "qa",
  id: "t1",
  question: "why?",
  answer: "because",
  presentation: [
    { tool: "highlight_lines", input: { file: "src/a.ts", start_line: 1, end_line: 1 } },
    { tool: "highlight_lines", input: { file: "src/a.ts", start_line: 2, end_line: 2 } },
  ],
});

describe("restoring an answer's effects", () => {
  it("offers the control only when the answer placed effects", () => {
    render(<ConversationTurn turn={{ kind: "qa", id: "t0", question: "q", answer: "a" }} />);
    expect(screen.queryByTestId("show-presentation")).toBeNull();
    cleanup();

    render(<ConversationTurn turn={answered()} onShowPresentation={() => undefined} />);
    expect(screen.getByTestId("show-presentation").textContent).toContain("(2)");
    cleanup();
  });

  it("reports a partial replay rather than claiming the whole answer is shown", async () => {
    const onShow = vi.fn().mockResolvedValue({ applied: 1, total: 2 });
    render(<ConversationTurn turn={answered()} onShowPresentation={onShow} />);

    fireEvent.click(screen.getByTestId("show-presentation"));

    await waitFor(() => expect(screen.getByText("1 of 2 shown")).toBeTruthy());
    expect(onShow).toHaveBeenCalledWith("t1", answered().presentation);
    cleanup();
  });

  it("replays recorded steps onto a clean page and counts what landed", async () => {
    document.body.innerHTML = FIXTURE;
    const m = createPresentationManager();

    const result = await m.replaySteps([
      { tool: "highlight_lines", input: { file: "src/a.ts", start_line: 1, end_line: 1 } },
      // A file the diff no longer shows: recorded, but it cannot land now.
      { tool: "highlight_lines", input: { file: "gone.ts", start_line: 1, end_line: 1 } },
    ]);

    expect(result).toEqual({ applied: 1, total: 2 });
    expect(document.querySelectorAll('[data-libre-cr-tag="highlight"]').length).toBe(1);

    // Showing another answer replaces the first — two answers' highlights on
    // one diff cannot be told apart.
    await m.replaySteps([
      { tool: "highlight_lines", input: { file: "src/a.ts", start_line: 2, end_line: 2 } },
    ]);
    const rows = document.querySelectorAll('[data-libre-cr-tag="highlight"]');
    expect(rows.length).toBe(1);
    expect(rows[0].querySelector("td")?.getAttribute("data-line-number")).toBe("2");
  });

  it("carries the recorded calls back from the session", () => {
    const turns = turnsFromSession([
      {
        turn_id: "t_1",
        kind: "question",
        status: "ok",
        question: "why?",
        answer: "because",
        presentation: [{ tool: "scroll_to", input: { file: "src/a.ts", line: 2 } }],
      },
      { turn_id: "t_2", kind: "question", status: "ok", question: "q", answer: "a" },
    ]);
    expect(turns[0].kind === "qa" && turns[0].presentation?.length).toBe(1);
    expect(turns[1].kind === "qa" && turns[1].presentation).toBeUndefined();
  });
});
