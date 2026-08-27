// Background service worker: relays daemon traffic for the content script.
//
// Content-script requests inherit the GitHub page's origin *and* CSP, whose
// `connect-src` excludes 127.0.0.1 — so the content script cannot reach the
// daemon directly (see `05-browser-extension.md` § Transport from a Content
// Script). This worker performs the fetch / holds the WebSocket instead; see
// `utils/daemon/proxy.ts` for the content-script half and message shapes.

import {
  FETCH_MSG,
  WS_PORT,
  type FetchRequestMsg,
  type FetchResponseMsg,
  type WsClientMsg,
  type WsServerMsg,
} from "../utils/daemon/proxy";

export default defineBackground(() => {
  browser.runtime.onMessage.addListener((raw: unknown, _sender, sendResponse) => {
    const msg = raw as Partial<FetchRequestMsg>;
    // Only relay messages are sent to this worker, so answering async (`true`)
    // unconditionally is safe and keeps the listener type simple.
    if (msg?.type !== FETCH_MSG || !msg.url) return true;
    (async () => {
      let res: FetchResponseMsg;
      try {
        const r = await fetch(msg.url!, { method: msg.method, headers: msg.headers, body: msg.body });
        res = { ok: true, status: r.status, statusText: r.statusText, headers: [...r.headers], body: await r.text() };
      } catch (e) {
        res = { ok: false, error: e instanceof Error ? e.message : String(e) };
      }
      sendResponse(res);
    })();
    return true; // async sendResponse
  });

  browser.runtime.onConnect.addListener((port) => {
    if (port.name !== WS_PORT) return;
    let ws: WebSocket | null = null;
    const post = (m: WsServerMsg) => {
      try {
        port.postMessage(m);
      } catch {
        ws?.close();
      }
    };
    port.onMessage.addListener((raw: unknown) => {
      const m = raw as WsClientMsg;
      if (m.t === "open") {
        try {
          ws = new WebSocket(m.url);
        } catch {
          post({ t: "error" });
          post({ t: "close", code: 1006, reason: "invalid websocket url" });
          return;
        }
        ws.onopen = () => post({ t: "open" });
        ws.onmessage = (e) => post({ t: "message", data: typeof e.data === "string" ? e.data : String(e.data) });
        ws.onerror = () => post({ t: "error" });
        ws.onclose = (e) => post({ t: "close", code: e.code, reason: e.reason });
      } else if (m.t === "send") {
        ws?.send(m.data);
      } else if (m.t === "close") {
        ws?.close(m.code, m.reason);
      }
    });
    port.onDisconnect.addListener(() => ws?.close());
  });
});
