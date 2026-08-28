// Content-script → background relay for daemon traffic.
//
// A content script's `fetch`/`WebSocket` run under the *page's* origin and
// CSP. GitHub's `connect-src` does not include 127.0.0.1, so direct calls
// die before leaving the browser ("Failed to fetch"). The background service
// worker is bound to neither, so the content script hands it the request.
// `DaemonClient` and `AskSession` take these via `fetch` / `wsFactory`.

import type { WSLike } from "./ws";

interface RuntimeLike {
  sendMessage(msg: unknown): Promise<unknown>;
  connect(info: { name: string }): PortLike;
}
export interface PortLike {
  postMessage(msg: unknown): void;
  disconnect(): void;
  onMessage: { addListener(cb: (msg: unknown) => void): void };
  onDisconnect: { addListener(cb: () => void): void };
}

export const FETCH_MSG = "libre-cr/fetch";
export const WS_PORT = "libre-cr/ws";
/** Well under Chrome's ~30 s service-worker idle limit. */
export const KEEPALIVE_MS = 20_000;

export interface FetchRequestMsg {
  type: typeof FETCH_MSG;
  url: string;
  method?: string;
  headers?: Record<string, string>;
  body?: string;
}
export type FetchResponseMsg =
  | { ok: true; status: number; statusText: string; headers: [string, string][]; body: string }
  | { ok: false; error: string };

export type WsClientMsg =
  | { t: "open"; url: string }
  | { t: "send"; data: string }
  | { t: "close"; code?: number; reason?: string }
  /** Keepalive: Chrome terminates an MV3 service worker after ~30 s without
   *  events, port included; a model thinking or a long tool call is silent
   *  for longer, and a dead worker drops the socket → the turn is cancelled. */
  | { t: "ping" };
export type WsServerMsg =
  | { t: "open" }
  | { t: "message"; data: string }
  | { t: "error" }
  | { t: "close"; code: number; reason: string };

function runtime(): RuntimeLike | null {
  const g = globalThis as unknown as { browser?: { runtime?: RuntimeLike }; chrome?: { runtime?: RuntimeLike } };
  const r = g.browser?.runtime ?? g.chrome?.runtime;
  return r && typeof r.sendMessage === "function" && typeof r.connect === "function" ? r : null;
}

/** `fetch` that runs in the background worker. Falls back to the global fetch
 *  outside an extension context (vitest, options-page preview). */
export function daemonFetch(): typeof fetch {
  const rt = runtime();
  if (!rt) return globalThis.fetch.bind(globalThis);
  return async (input, init = {}) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const headers: Record<string, string> = {};
    new Headers(init.headers).forEach((v, k) => (headers[k] = v));
    const msg: FetchRequestMsg = {
      type: FETCH_MSG,
      url,
      method: init.method,
      headers,
      body: typeof init.body === "string" ? init.body : undefined,
    };
    const res = (await rt.sendMessage(msg)) as FetchResponseMsg | undefined;
    if (!res) throw new TypeError("background worker did not respond");
    if (!res.ok) throw new TypeError(res.error);
    // `Response` refuses a body on null-body statuses.
    const body = [101, 204, 205, 304].includes(res.status) ? null : res.body;
    return new Response(body, { status: res.status, statusText: res.statusText, headers: res.headers });
  };
}

class ProxyWebSocket implements WSLike {
  readyState = 0; // CONNECTING
  onopen: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  private port: PortLike;
  private keepalive: ReturnType<typeof setInterval> | null = null;

  constructor(rt: RuntimeLike, url: string) {
    this.port = rt.connect({ name: WS_PORT });
    this.port.onMessage.addListener((raw) => {
      const m = raw as WsServerMsg;
      switch (m.t) {
        case "open":
          this.readyState = 1;
          this.onopen?.(new Event("open"));
          break;
        case "message":
          this.onmessage?.(new MessageEvent("message", { data: m.data }));
          break;
        case "error":
          this.onerror?.(new Event("error"));
          break;
        case "close":
          this.finish(m.code, m.reason);
          break;
      }
    });
    // Worker died or port dropped: surface as an abnormal close.
    this.port.onDisconnect.addListener(() => this.finish(1006, "background port disconnected"));
    this.post({ t: "open", url });
    this.keepalive = setInterval(() => this.post({ t: "ping" }), KEEPALIVE_MS);
  }
  send(data: string): void {
    this.post({ t: "send", data });
  }
  close(code?: number, reason?: string): void {
    if (this.readyState >= 2) return;
    this.readyState = 2;
    if (this.keepalive) clearInterval(this.keepalive);
    this.keepalive = null;
    this.post({ t: "close", code, reason });
  }
  private post(m: WsClientMsg): void {
    try {
      this.port.postMessage(m);
    } catch {
      this.finish(1006, "background port unavailable");
    }
  }
  private finish(code: number, reason: string): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    if (this.keepalive) clearInterval(this.keepalive);
    this.keepalive = null;
    try {
      this.port.disconnect();
    } catch {
      /* already gone */
    }
    this.onclose?.(new CloseEvent("close", { code, reason }));
  }
}

/** `wsFactory` for `AskSession`: the real socket lives in the background worker. */
export function daemonWsFactory(): ((url: string) => WSLike) | undefined {
  const rt = runtime();
  return rt ? (url) => new ProxyWebSocket(rt, url) : undefined;
}
