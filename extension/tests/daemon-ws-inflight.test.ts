import { describe, expect, it } from "vitest";

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
  fireOpen() {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }
  fireError() {
    this.onerror?.(new Event("error"));
  }
  fireMessage(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) }));
  }
}

describe("AskSession — inflight lifecycle (C7)", () => {
  it("clears inflight on error so the session is reusable", async () => {
    let mock = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock,
    });

    const p = sess.open({ question: "boom" });
    mock.fireOpen();
    // Error before any `done` frame.
    mock.fireError();
    await expect(p).rejects.toThrow(/websocket error/);

    // Caller may forget the finally — the session must still be reusable.
    // A second `open()` should not throw "already in flight".
    mock = new MockWS();
    const sess2Promise = sess.open({ question: "retry" });
    mock.fireOpen();
    mock.fireMessage({
      type: "done",
      turn_id: "t2",
      usage: { input_tokens: 0, output_tokens: 0 },
    });
    await expect(sess2Promise).resolves.toBeUndefined();
  });

  it("clears inflight on close before open arrives", async () => {
    const mock = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock,
    });
    const p = sess.open({ question: "x" });
    // Close before `onopen` ever fired.
    mock.close();
    await expect(p).rejects.toThrow(/closed before init/);

    // Reusable.
    const mock2 = new MockWS();
    (sess as unknown as { options: { wsFactory: () => WSLike } }).options.wsFactory = () => mock2;
    const p2 = sess.open({ question: "again" });
    mock2.fireOpen();
    mock2.fireMessage({
      type: "done",
      turn_id: "t",
      usage: { input_tokens: 0, output_tokens: 0 },
    });
    await expect(p2).resolves.toBeUndefined();
  });

  it("a synchronous wsFactory throw resets inflight and allows a retry (C7 residual)", async () => {
    let calls = 0;
    const good = new MockWS();
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => {
        calls++;
        if (calls === 1) throw new Error("ctor boom");
        return good;
      },
    });

    await expect(sess.open({ question: "x" })).rejects.toThrow(/ctor boom/);
    // The session must not be stranded in-flight after the constructor throw.
    expect((sess as unknown as { inflight: boolean }).inflight).toBe(false);
    expect((sess as unknown as { socket: unknown }).socket).toBeNull();

    // A retry works — does not throw "already in flight".
    const p = sess.open({ question: "retry" });
    good.fireOpen();
    good.fireMessage({
      type: "done",
      turn_id: "t",
      usage: { input_tokens: 0, output_tokens: 0 },
    });
    await expect(p).resolves.toBeUndefined();
  });

  it("close() is idempotent — multiple calls do not throw or re-fire", () => {
    let closeCount = 0;
    const mock: WSLike = {
      readyState: 1,
      onopen: null,
      onclose: null,
      onerror: null,
      onmessage: null,
      send() {},
      close() {
        closeCount++;
      },
    };
    const sess = new AskSession("http://127.0.0.1:5", "tk", "sid", {
      wsFactory: () => mock,
    });
    // Drive open() partly so socket is assigned.
    void sess.open({ question: "q" });
    sess.close();
    sess.close();
    sess.close();
    expect(closeCount).toBe(1);
  });
});
