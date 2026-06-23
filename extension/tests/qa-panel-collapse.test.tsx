// E2 (round 2): auto-collapse of older turns. Collapse is a controlled prop —
// QaPanel owns `collapsed` on each turn, marks older qa turns collapsed when a
// new question lands, and ConversationTurn renders straight from the prop.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { QaPanel } from "../components/QaPanel";

const { fakeSessions } = vi.hoisted(() => ({
  fakeSessions: [] as Array<{
    emit: (frame: { type: string; [k: string]: unknown }) => void;
    finish: () => void;
  }>,
}));

vi.mock("../utils/daemon/ws", async (importOriginal) => {
  const original = await importOriginal<typeof import("../utils/daemon/ws")>();
  class FakeAskSession {
    handlers = new Map<string, Set<(f: unknown) => void>>();
    private resolveOpen: (() => void) | null = null;

    on(type: string, h: (f: unknown) => void) {
      let set = this.handlers.get(type);
      if (!set) {
        set = new Set();
        this.handlers.set(type, set);
      }
      set.add(h);
      return () => set!.delete(h);
    }
    onAny() {
      return () => {};
    }
    onClose() {
      return () => {};
    }
    open(): Promise<void> {
      fakeSessions.push({
        emit: (frame) => {
          for (const h of this.handlers.get(frame.type) ?? []) h(frame);
        },
        finish: () => this.resolveOpen?.(),
      });
      return new Promise<void>((r) => {
        this.resolveOpen = r;
      });
    }
    sendPresentationResult() {}
    close() {}
  }
  return { ...original, AskSession: FakeAskSession };
});

function mockClient(): DaemonClient {
  const fetchFn = vi.fn().mockImplementation((url: string) => {
    if (url.endsWith("/v1/verbs")) {
      return Promise.resolve(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    }
    return Promise.resolve(new Response("{}", { status: 200 }));
  });
  return new DaemonClient(
    { endpoint: "http://127.0.0.1:1", token: "t" },
    { fetch: fetchFn as unknown as typeof fetch },
  );
}

async function ask(question: string, answer: string): Promise<void> {
  const before = fakeSessions.length;
  const textarea = screen.getByPlaceholderText(/Ask a question/);
  fireEvent.change(textarea, { target: { value: question } });
  fireEvent.click(screen.getByText(/Ask ▶/));
  await waitFor(() => expect(fakeSessions.length).toBe(before + 1));
  const sess = fakeSessions[before];
  await act(async () => {
    sess.emit({ type: "text_delta", text: answer });
    sess.finish();
  });
  // Turn settles: pending=false, asking=false (Ask button re-enabled).
  await waitFor(() => expect(screen.getByText(/Ask ▶/)).toBeTruthy());
}

describe("QaPanel — auto-collapse of older turns (E2)", () => {
  afterEach(() => {
    cleanup();
    fakeSessions.length = 0;
  });

  it("collapses older turns when a new question is asked; clicking expands", async () => {
    render(
      <QaPanel
        client={mockClient()}
        sessionId="sess-collapse"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    await screen.findByTestId("empty-state");

    await ask("first question", "first answer");
    await ask("second question", "second answer");
    await ask("third question", "third answer");

    // Three qa turns rendered; the two older ones are collapsed summaries,
    // the newest is expanded (full answer visible, no summary).
    expect(screen.getAllByTestId("qa-turn").length).toBe(3);
    const collapsed = screen.getAllByTestId("expand-qa");
    expect(collapsed.length).toBe(2);
    expect(collapsed[0].textContent).toContain("first answer");
    expect(collapsed[1].textContent).toContain("second answer");
    expect(screen.getByText("third answer")).toBeTruthy();
    // Older answers only appear inside collapsed summaries.
    expect(screen.queryByText("first answer")).toBeNull();

    // Clicking an older turn's summary expands it again.
    fireEvent.click(collapsed[0]);
    await waitFor(() => {
      expect(screen.getAllByTestId("expand-qa").length).toBe(1);
      expect(screen.getByText("first answer")).toBeTruthy();
    });
  });
});
