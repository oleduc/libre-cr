import { afterEach, describe, expect, it, vi } from "vitest";

import { AskSession, type WSLike } from "../utils/daemon/ws";

class MockWS implements WSLike {
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  sent: string[] = [];
  send(s: string) {
    this.sent.push(s);
  }
  close() {
    this.readyState = 3;
    this.onclose?.(new CloseEvent("close"));
  }
  // Test helpers
  fireOpen() {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }
  fireMessage(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) }));
  }
}

describe("AskSession", () => {
  let mock: MockWS | null = null;
  afterEach(() => {
    mock?.close();
    mock = null;
  });

  it("sends the AskInit frame after open and dispatches events", async () => {
    mock = new MockWS();
    const factory = () => mock!;
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", { wsFactory: factory });
    const seen: string[] = [];
    sess.onAny((f) => seen.push(f.type));
    const p = sess.open({ question: "why?" });
    // Open + stream a few frames + done.
    mock.fireOpen();
    expect(mock.sent.length).toBe(1);
    const init = JSON.parse(mock.sent[0]!);
    expect(init.question).toBe("why?");
    mock.fireMessage({ type: "text_delta", text: "hello" });
    mock.fireMessage({
      type: "tool_call",
      call_id: "c1",
      name: "find_references",
      input: {},
    });
    mock.fireMessage({
      type: "done",
      turn_id: "t1",
      usage: { input_tokens: 1, output_tokens: 1 },
    });
    await p;
    expect(seen).toEqual(["text_delta", "tool_call", "done"]);
  });

  it("rejects when the server emits an error frame", async () => {
    mock = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock!,
    });
    const p = sess.open({ question: "boom" });
    mock.fireOpen();
    mock.fireMessage({ type: "error", message: "provider down", recoverable: false });
    await expect(p).rejects.toThrow("provider down");
  });

  it("ignores unknown frame shapes without crashing", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mock = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock!,
    });
    const seen: string[] = [];
    sess.onAny((f) => seen.push(f.type));
    const p = sess.open({ question: "ok" });
    mock.fireOpen();
    mock.fireMessage({ type: "not_a_real_frame", text: "garbage" });
    mock.fireMessage({
      type: "done",
      turn_id: "t1",
      usage: { input_tokens: 0, output_tokens: 0 },
    });
    await p;
    expect(seen).toEqual(["done"]);
    warn.mockRestore();
  });

  it("sends presentation_result when asked", async () => {
    mock = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock!,
    });
    const p = sess.open({ question: "hi" });
    mock.fireOpen();
    sess.sendPresentationResult("p1", true, { effect_id: "e1" });
    expect(mock.sent.length).toBe(2);
    const second = JSON.parse(mock.sent[1]!);
    expect(second.type).toBe("presentation_result");
    expect(second.call_id).toBe("p1");
    expect(second.ok).toBe(true);
    mock.fireMessage({
      type: "done",
      turn_id: "t",
      usage: { input_tokens: 0, output_tokens: 0 },
    });
    await p;
  });
});
