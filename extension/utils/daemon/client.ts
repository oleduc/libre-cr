// Typed HTTP client for the review daemon.
//
// Implements the routes from `04-review-daemon.md` § HTTP API. Uses bearer
// auth and surfaces structured errors that mirror the daemon's
// `ErrorEnvelope` shape.

import type { Selection } from "../selection";
import type {
  CreateSessionResponse,
  ErrorCategory,
  ErrorEnvelope,
  GetSessionResponse,
  PairResponse,
  SessionSummary,
  VerbDescriptor,
} from "./frames";
import { recordProtocolCheck } from "./protocol";

export interface DaemonAuth {
  endpoint: string;
  token: string;
}

export interface ScrapedPRData {
  owner: string | null;
  repo: string | null;
  number: number | null;
  title: string | null;
  description: string | null;
  author: string | null;
  base_branch: string | null;
  head_branch: string | null;
  head_sha: string | null;
  files_changed: string[];
}

export class DaemonError extends Error {
  public status: number;
  public category: ErrorCategory | "transport" | "parse";
  public envelope?: ErrorEnvelope;

  constructor(
    message: string,
    status: number,
    category: ErrorCategory | "transport" | "parse",
    envelope?: ErrorEnvelope,
  ) {
    super(message);
    this.name = "DaemonError";
    this.status = status;
    this.category = category;
    this.envelope = envelope;
  }
}

type FetchFn = typeof fetch;

export interface ClientOptions {
  fetch?: FetchFn;
  signal?: AbortSignal;
}

/**
 * Bare HTTP transport. One method per route. All errors are normalized into
 * `DaemonError` instances with a `category` derived from the daemon's
 * `ErrorEnvelope` when available.
 */
export class DaemonClient {
  private fetchFn: FetchFn;

  constructor(
    public auth: DaemonAuth,
    options: ClientOptions = {},
  ) {
    this.fetchFn = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  setAuth(auth: DaemonAuth): void {
    this.auth = auth;
  }

  async request<T>(
    method: string,
    path: string,
    body?: unknown,
    init: RequestInit = {},
  ): Promise<T> {
    const url = `${this.auth.endpoint.replace(/\/$/, "")}${path}`;
    let resp: Response;
    const headers: Record<string, string> = {
      Accept: "application/json",
      ...(init.headers as Record<string, string> | undefined),
    };
    if (this.auth.token) {
      headers.Authorization = `Bearer ${this.auth.token}`;
    }
    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
    }
    try {
      resp = await this.fetchFn(url, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        ...init,
        // Don't let caller's `headers` clobber the merged ones.
        // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      });
    } catch (e) {
      throw new DaemonError(
        e instanceof Error ? e.message : "network error",
        0,
        "transport",
      );
    }
    if (!resp.ok) {
      let env: ErrorEnvelope | undefined;
      let raw: string | undefined;
      try {
        raw = await resp.text();
        if (raw) env = JSON.parse(raw) as ErrorEnvelope;
      } catch {
        // ignore — keep raw text below
      }
      const category: ErrorCategory | "transport" =
        env?.error ??
        (resp.status === 401
          ? "unauthorized"
          : resp.status === 403
            ? "origin_rejected"
            : "internal");
      throw new DaemonError(
        env?.message ?? raw ?? `HTTP ${resp.status}`,
        resp.status,
        category,
        env,
      );
    }
    if (resp.status === 204) return undefined as T;
    try {
      return (await resp.json()) as T;
    } catch (e) {
      throw new DaemonError(
        e instanceof Error ? e.message : "invalid JSON",
        resp.status,
        "parse",
      );
    }
  }

  async getHealth(): Promise<{
    ok: boolean;
    version: string;
    protocol_version?: number;
  }> {
    const health = await this.request<{
      ok: boolean;
      version: string;
      protocol_version?: number;
    }>("GET", "/v1/health");
    // Soft check — warns + records `ui.protocol_mismatch`, never throws.
    await recordProtocolCheck(health);
    return health;
  }

  createOrUpdateSession(
    pr_url: string,
    pr_data: ScrapedPRData | Record<string, unknown>,
  ): Promise<CreateSessionResponse> {
    return this.request("POST", "/v1/sessions", { pr_url, pr_data });
  }

  getSession(id: string): Promise<GetSessionResponse> {
    return this.request("GET", `/v1/sessions/${encodeURIComponent(id)}`);
  }

  listSessions(
    limit = 50,
  ): Promise<{ sessions: SessionSummary[] }> {
    return this.request(
      "GET",
      `/v1/sessions?limit=${encodeURIComponent(String(limit))}`,
    );
  }

  deleteSession(id: string): Promise<void> {
    return this.request("DELETE", `/v1/sessions/${encodeURIComponent(id)}`);
  }

  addNote(
    id: string,
    content: string,
    anchor?: Selection,
    severity?: string,
    source_turn_id?: string,
  ): Promise<{ note_id: string }> {
    return this.request(
      "POST",
      `/v1/sessions/${encodeURIComponent(id)}/notes`,
      { content, anchor, severity, source_turn_id },
    );
  }

  search(
    q: string,
    limit = 20,
  ): Promise<{
    results: Array<{
      session_id: string;
      pr_url: string;
      turn_id: string;
      snippet: string;
      score: number;
    }>;
  }> {
    const url = `/v1/search?q=${encodeURIComponent(q)}&limit=${encodeURIComponent(
      String(limit),
    )}`;
    return this.request("GET", url);
  }

  updateNote(
    id: string,
    note_id: string,
    body: { content?: string; severity?: string },
  ): Promise<void> {
    return this.request(
      "PATCH",
      `/v1/sessions/${encodeURIComponent(id)}/notes/${encodeURIComponent(note_id)}`,
      body,
    );
  }

  deleteNote(id: string, note_id: string): Promise<void> {
    return this.request(
      "DELETE",
      `/v1/sessions/${encodeURIComponent(id)}/notes/${encodeURIComponent(note_id)}`,
    );
  }

  exportSession(
    id: string,
    body: Record<string, unknown>,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/v1/sessions/${encodeURIComponent(id)}/export`,
      body,
    );
  }

  getConfig(): Promise<unknown> {
    return this.request("GET", "/v1/config");
  }

  validateConfig(): Promise<{ ok: boolean }> {
    return this.request("POST", "/v1/config/validate", {});
  }

  pair(code: string, extension_origin?: string): Promise<PairResponse> {
    return this.request("POST", "/v1/pair", { code, extension_origin });
  }

  getVerbs(): Promise<VerbDescriptor[]> {
    return this.request("GET", "/v1/verbs");
  }
}
