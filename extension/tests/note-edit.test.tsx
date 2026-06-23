import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { ConversationTurn, severityGlyph } from "../components/ConversationTurn";

describe("ConversationTurn note rendering + edit", () => {
  afterEach(() => cleanup());

  it("renders a severity glyph beside the note", () => {
    render(
      <ConversationTurn
        turn={{
          kind: "note",
          id: "n1",
          noteId: "n1",
          content: "looks fine",
          severity: "warning",
        }}
      />,
    );
    expect(screen.getByLabelText(/severity warning/)).toBeTruthy();
  });

  it("severityGlyph maps to a stable icon set", () => {
    expect(severityGlyph("info")).toBeTruthy();
    expect(severityGlyph("critical")).toBeTruthy();
  });

  it("clicking Edit opens an inline editor and Save invokes onEditNote", async () => {
    const onEditNote = vi.fn().mockResolvedValue(undefined);
    render(
      <ConversationTurn
        turn={{
          kind: "note",
          id: "n1",
          noteId: "n1",
          content: "old text",
          severity: "info",
        }}
        onEditNote={onEditNote}
      />,
    );
    fireEvent.click(screen.getByTestId("edit-note"));
    const textarea = screen.getByTestId("edit-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("old text");
    fireEvent.change(textarea, { target: { value: "updated" } });
    fireEvent.change(screen.getByTestId("edit-severity"), {
      target: { value: "critical" },
    });
    fireEvent.click(screen.getByText("Save"));
    await Promise.resolve();
    expect(onEditNote).toHaveBeenCalledWith("n1", "updated", "critical");
  });

  it("requires a second click on Delete to confirm", async () => {
    const onDeleteNote = vi.fn().mockResolvedValue(undefined);
    render(
      <ConversationTurn
        turn={{
          kind: "note",
          id: "n1",
          noteId: "n2",
          content: "x",
          severity: "info",
        }}
        onDeleteNote={onDeleteNote}
      />,
    );
    fireEvent.click(screen.getByTestId("delete-note"));
    expect(onDeleteNote).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("confirm-delete"));
    expect(onDeleteNote).toHaveBeenCalledWith("n2");
  });
});
