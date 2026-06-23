import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { QaPanel } from "../components/QaPanel";
import { ExportModal } from "../components/ExportModal";
import { getKey } from "../utils/daemon/storage";

// Controllable stand-in for `AskSession` so tests can feed server frames
// (e.g. `presentation_call`) into a mounted QaPanel.
const { fakeSessions } = vi.hoisted(() => ({
  fakeSessions: [] as Array<{
    init: unknown;
    sent: Array<{ call_id: string; ok: boolean; error?: string }>;
    emit: (frame: { type: string; [k: string]: unknown }) => void;
    finish: () => void;
  }>,
}));

vi.mock("../utils/daemon/ws", async (importOriginal) => {
  const original = await importOriginal<typeof import("../utils/daemon/ws")>();
  class FakeAskSession {
    handlers = new Map<string, Set<(f: unknown) => void>>();
    sent: Array<{ call_id: string; ok: boolean; error?: string }> = [];
    init: unknown = null;
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
    open(init: unknown): Promise<void> {
      this.init = init;
      fakeSessions.push({
        init,
        sent: this.sent,
        emit: (frame) => {
          for (const h of this.handlers.get(frame.type) ?? []) h(frame);
        },
        finish: () => this.resolveOpen?.(),
      });
      return new Promise<void>((r) => {
        this.resolveOpen = r;
      });
    }
    sendPresentationResult(call_id: string, ok: boolean, _result?: unknown, error?: string) {
      this.sent.push({ call_id, ok, error });
    }
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

describe("QaPanel — aria-live + mute toggle (I16/I18)", () => {
  afterEach(() => cleanup());

  it("conversation pane is annotated as a polite live region", async () => {
    const { container } = render(
      <QaPanel
        client={mockClient()}
        sessionId="s"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    const conv = container.querySelector(".libre-cr-conversation");
    expect(conv).not.toBeNull();
    expect(conv!.getAttribute("aria-live")).toBe("polite");
    expect(conv!.getAttribute("role")).toBe("log");
  });

  it("clicking the mute toggle persists the per-session preference", async () => {
    render(
      <QaPanel
        client={mockClient()}
        sessionId="sess-mute"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    const btn = await screen.findByTestId("toggle-mute-presentations");
    expect(btn.getAttribute("aria-pressed")).toBe("false");
    expect(btn.textContent).toBe("🔊");

    fireEvent.click(btn);
    expect(btn.getAttribute("aria-pressed")).toBe("true");
    expect(btn.textContent).toBe("🔇");

    await waitFor(async () => {
      const map = (await getKey("session.presentations_muted")) ?? {};
      expect(map["sess-mute"]).toBe(true);
    });
  });

  it("muted session receiving a presentation_call applies no DOM effect (E1)", async () => {
    fakeSessions.length = 0;
    // Page-DOM diff fixture that `highlight_lines` would tag if executed.
    const container = document.createElement("div");
    container.setAttribute("data-tagsearch-path", "src/main.rs");
    container.innerHTML =
      '<table><tbody><tr><td class="blob-num" data-line-number="1"></td>' +
      '<td class="blob-code">fn main() {}</td></tr></tbody></table>';
    document.body.appendChild(container);

    render(
      <QaPanel
        client={mockClient()}
        sessionId="sess-mute-gate"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );

    // Mute, then ask a question.
    fireEvent.click(await screen.findByTestId("toggle-mute-presentations"));
    const textarea = screen.getByPlaceholderText(/Ask a question/);
    fireEvent.change(textarea, { target: { value: "highlight something" } });
    fireEvent.click(screen.getByText(/Ask ▶/));

    await waitFor(() => expect(fakeSessions.length).toBe(1));
    const sess = fakeSessions[0];
    // The AskInit carries the daemon-side flag too.
    expect((sess.init as { mute_presentations?: boolean }).mute_presentations).toBe(true);

    // Daemon misbehaves (ignores the flag) and emits a presentation call.
    sess.emit({
      type: "presentation_call",
      call_id: "c1",
      tool: "highlight_lines",
      input: { file: "src/main.rs", start_line: 1, end_line: 1 },
    });

    // Local gate: no DOM effect applied, call answered with presentation_muted.
    expect(document.querySelectorAll("[data-libre-cr-effect-id]").length).toBe(0);
    expect(sess.sent).toEqual([
      { call_id: "c1", ok: false, error: "presentation_muted" },
    ]);

    await act(async () => sess.finish());
    container.remove();
  });
});

describe("ExportModal — focus trap & dialog semantics (I18)", () => {
  afterEach(() => cleanup());

  it("has role=dialog, aria-modal, aria-labelledby pointing at the title", () => {
    const client = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: (() => Promise.resolve(new Response("{}", { status: 200 }))) as unknown as typeof fetch },
    );
    const { container } = render(
      <ExportModal client={client} sessionId="s1" onClose={() => {}} />,
    );
    const dialog = container.querySelector('[role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog!.getAttribute("aria-modal")).toBe("true");
    const labelledBy = dialog!.getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    const title = document.getElementById(labelledBy!);
    expect(title).not.toBeNull();
    expect(title!.textContent).toMatch(/Export review draft/);
  });

  it("closes when Escape is pressed", () => {
    const client = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: (() => Promise.resolve(new Response("{}", { status: 200 }))) as unknown as typeof fetch },
    );
    const onClose = vi.fn();
    render(<ExportModal client={client} sessionId="s1" onClose={onClose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
