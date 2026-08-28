// Typed WebSocket session against `/v1/sessions/:id/ask`.
//
// Single in-flight per `Session` instance. Auto-reconnect is out of scope —
// disconnecting terminates the current turn.

import type { Selection } from "../selection";
import type { AskInit, ClientFrame, ServerFrame } from "./frames";
import { parseServerFrame } from "./frames";

export type ServerFrameType = ServerFrame["type"];

type Handler<F extends ServerFrame> = (frame: F) => void;
type AnyHandler = (frame: ServerFrame) => void;

/**
 * Minimal interface for a `WebSocket`-like object so tests can swap in a
 * fake without depending on a global polyfill.
 */
export interface WSLike {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: Event) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  onerror: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
}

export interface SessionOptions {
  /** Override `WebSocket` constructor (tests, background-script proxy). */
  wsFactory?: (url: string) => WSLike;
}

export class AskSession {
  private socket: WSLike | null = null;
  private handlers = new Map<ServerFrameType, Set<AnyHandler>>();
  private onAnyHandlers = new Set<AnyHandler>();
  private closeHandlers = new Set<(reason?: string) => void>();
  private opened = false;
  private inflight = false;
  private closed = false;

  constructor(
    private endpoint: string,
    private token: string,
    private sessionId: string,
    private options: SessionOptions = {},
  ) {}

  on<T extends ServerFrameType>(
    type: T,
    handler: Handler<Extract<ServerFrame, { type: T }>>,
  ): () => void {
    let set = this.handlers.get(type);
    if (!set) {
      set = new Set();
      this.handlers.set(type, set);
    }
    set.add(handler as AnyHandler);
    return () => set!.delete(handler as AnyHandler);
  }

  onAny(handler: AnyHandler): () => void {
    this.onAnyHandlers.add(handler);
    return () => this.onAnyHandlers.delete(handler);
  }

  onClose(handler: (reason?: string) => void): () => void {
    this.closeHandlers.add(handler);
    return () => this.closeHandlers.delete(handler);
  }

  private deliver(frame: ServerFrame): void {
    const set = this.handlers.get(frame.type);
    if (set) {
      for (const h of set) h(frame);
    }
    for (const h of this.onAnyHandlers) h(frame);
  }

  /**
   * Open the WS and send the first `AskInit` frame. Resolves when the
   * `done` frame arrives or rejects on error / unexpected close.
   */
  async open(init: AskInit): Promise<void> {
    if (this.inflight) {
      throw new Error("AskSession is already in flight");
    }
    this.inflight = true;
    this.closed = false;
    this.opened = false;
    const url = this.buildUrl();
    let ws: WSLike;
    try {
      ws = this.options.wsFactory
        ? this.options.wsFactory(url)
        : (new WebSocket(url) as unknown as WSLike);
    } catch (e) {
      // A synchronous constructor throw (invalid URL, factory failure) must
      // not strand the session in-flight — reset state and reject (C7
      // residual from round 2).
      this.inflight = false;
      this.opened = false;
      this.socket = null;
      throw e instanceof Error ? e : new Error(String(e));
    }
    this.socket = ws;

    return new Promise<void>((resolve, reject) => {
      let settled = false;
      const settle = (err?: Error) => {
        // Clearing `inflight` here (in addition to `close()`) guarantees the
        // session is reusable even if the caller bypasses its `finally` —
        // e.g. an `error` event before `open`, or an unexpected throw.
        this.inflight = false;
        if (settled) return;
        settled = true;
        if (err) reject(err);
        else resolve();
      };

      ws.onopen = () => {
        this.opened = true;
        try {
          ws.send(JSON.stringify(init satisfies AskInit));
        } catch (e) {
          settle(e instanceof Error ? e : new Error(String(e)));
        }
      };
      let finished = false; // `done` or `error` frame seen
      ws.onmessage = (ev: MessageEvent) => {
        const raw = typeof ev.data === "string" ? ev.data : String(ev.data);
        const frame = parseServerFrame(raw);
        if (!frame) {
          console.warn("[libre-cr] dropping unknown frame:", raw.slice(0, 120));
          return;
        }
        this.deliver(frame);
        if (frame.type === "done") {
          finished = true;
          settle();
        } else if (frame.type === "error") {
          finished = true;
          settle(new Error(frame.message));
        }
      };
      ws.onerror = () => {
        settle(new Error("websocket error"));
      };
      ws.onclose = (ev: CloseEvent) => {
        for (const h of this.closeHandlers) h(ev.reason);
        // A close without `done` is a failure whether or not the socket had
        // opened: a daemon dying mid-stream used to look like a finished turn
        // with an empty answer.
        if (!finished) {
          settle(
            new Error(
              this.opened
                ? "connection closed before the answer completed (daemon stopped?)"
                : "websocket closed before init",
            ),
          );
        }
      };
    });
  }

  private buildUrl(): string {
    const wsBase = this.endpoint
      .replace(/^http:\/\//, "ws://")
      .replace(/^https:\/\//, "wss://")
      .replace(/\/$/, "");
    // Token is passed as a query param because browsers don't let us set
    // Authorization headers on `new WebSocket(...)`. Daemon's auth middleware
    // accepts the `?token=` fallback for the WS upgrade.
    const t = encodeURIComponent(this.token);
    const id = encodeURIComponent(this.sessionId);
    return `${wsBase}/v1/sessions/${id}/ask?token=${t}`;
  }

  /**
   * Send a `presentation_result` reply.
   */
  sendPresentationResult(
    call_id: string,
    ok: boolean,
    result?: Record<string, unknown>,
    error?: string,
    message?: string,
  ): void {
    if (!this.socket || this.socket.readyState !== 1) {
      // 1 === OPEN
      return;
    }
    const frame: ClientFrame = {
      type: "presentation_result",
      call_id,
      ok,
      ...(result !== undefined ? { result } : {}),
      ...(error !== undefined ? { error } : {}),
      ...(message !== undefined ? { message } : {}),
    };
    this.socket.send(JSON.stringify(frame));
  }

  /** Idempotent — safe to call multiple times. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.socket) {
      try {
        this.socket.close();
      } catch {
        // ignore
      }
      this.socket = null;
    }
    this.inflight = false;
  }
}
