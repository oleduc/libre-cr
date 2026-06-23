import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { ConversationTurn } from "../components/ConversationTurn";

describe("ConversationTurn save-as-note", () => {
  afterEach(() => cleanup());

  it("opens the overlay, lets the user pick severity, and calls onSaveAsNote with the chosen values", async () => {
    const onSaveAsNote = vi.fn().mockResolvedValue(undefined);
    render(
      <ConversationTurn
        turn={{
          kind: "qa",
          id: "t_src_1",
          question: "where is bcryptHash used?",
          answer: "Found 1 reference in src/auth/legacy.ts:88.",
        }}
        onSaveAsNote={onSaveAsNote}
      />,
    );
    fireEvent.click(screen.getByTestId("save-as-note"));
    // Overlay is open with answer pre-filled.
    const textarea = screen.getByTestId("save-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toContain("legacy.ts:88");
    // Change severity.
    fireEvent.change(screen.getByTestId("save-severity"), {
      target: { value: "warning" },
    });
    // Edit content.
    fireEvent.change(textarea, { target: { value: "Pinned: legacy md5 path" } });
    // Confirm.
    fireEvent.click(screen.getByTestId("save-as-note-confirm"));
    await Promise.resolve();
    expect(onSaveAsNote).toHaveBeenCalledWith(
      "t_src_1",
      "Pinned: legacy md5 path",
      "warning",
    );
  });

  it("DaemonClient.addNote includes source_turn_id when provided", async () => {
    const fetchFn = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ note_id: "n1" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    await c.addNote("sess1", "hello", undefined, "warning", "t_src_xyz");
    const [, init] = fetchFn.mock.calls[0]!;
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body).toMatchObject({
      content: "hello",
      severity: "warning",
      source_turn_id: "t_src_xyz",
    });
  });
});
