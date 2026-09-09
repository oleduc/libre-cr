// Exceeding a context cap deliberately does NOT fail the answer — the daemon
// truncates the tool result and reports it — so this visible notice is the only
// thing that stops a shortened answer from looking complete.
//
// The caps themselves are edited in the daemon's own config UI (/config-ui),
// not here: the daemon enforces them and must keep them without the extension.

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

import { ConversationTurn, type Turn } from "../components/ConversationTurn";


describe("truncation notice", () => {
  afterEach(() => cleanup());

  const turnWith = (thinking: Turn extends { thinking?: infer T } ? T : never): Turn => ({
    kind: "qa",
    id: "t1",
    question: "what does this do?",
    answer: "It does the thing.",
    thinking,
  });

  it("tells the reviewer when a result was shortened, and by how much", () => {
    render(
      <ConversationTurn
        turn={turnWith([
          { call_id: "c1", name: "get_pr_diff", truncatedFrom: 589499 },
          { call_id: "c2", name: "read_file" },
        ])}
      />,
    );
    const notice = screen.getByTestId("truncation-notice");
    expect(notice.textContent).toMatch(/1 tool result hit the context limits/);
    expect(notice.textContent).toMatch(/589,499 chars/);
    // And the individual trace is marked, so it's clear which call was cut.
    expect(screen.getByText(/truncated from 589,499 chars/)).toBeTruthy();
  });

  it("stays silent when everything fit", () => {
    render(<ConversationTurn turn={turnWith([{ call_id: "c1", name: "grep" }])} />);
    expect(screen.queryByTestId("truncation-notice")).toBeNull();
  });

  it("counts every shortened result", () => {
    render(
      <ConversationTurn
        turn={turnWith([
          { call_id: "c1", name: "get_pr_diff", truncatedFrom: 500000 },
          { call_id: "c2", name: "grep", truncatedFrom: 31259 },
        ])}
      />,
    );
    expect(screen.getByTestId("truncation-notice").textContent).toMatch(
      /2 tool results hit the context limits and were shortened/,
    );
  });
});
