import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { KEEPALIVE_MS, daemonWsFactory } from "../utils/daemon/proxy";

/** Fake `chrome.runtime` whose port records what the content script posts. */
function installFakeRuntime() {
  const posted: unknown[] = [];
  const port = {
    postMessage: (m: unknown) => posted.push(m),
    disconnect: vi.fn(),
    onMessage: { addListener: vi.fn() },
    onDisconnect: { addListener: vi.fn() },
  };
  (globalThis as unknown as { chrome?: unknown }).chrome = {
    runtime: { sendMessage: vi.fn(), connect: () => port },
  };
  return { posted, port };
}

describe("relayed WebSocket keepalive", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    delete (globalThis as unknown as { chrome?: unknown }).chrome;
  });

  it("pings the background port while the socket is open and stops on close", () => {
    const { posted } = installFakeRuntime();
    const ws = daemonWsFactory()!("ws://127.0.0.1:1/v1/sessions/s/ask");
    expect(posted[0]).toMatchObject({ t: "open" });
    vi.advanceTimersByTime(KEEPALIVE_MS * 2 + 5);
    expect(posted.filter((m) => (m as { t: string }).t === "ping").length).toBe(2);
    ws.close();
    vi.advanceTimersByTime(KEEPALIVE_MS * 3);
    expect(posted.filter((m) => (m as { t: string }).t === "ping").length).toBe(2);
  });
});
