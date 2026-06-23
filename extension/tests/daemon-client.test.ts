import { describe, expect, it, vi } from "vitest";

import { DaemonClient, DaemonError } from "../utils/daemon/client";

function jsonResponse(body: unknown, init: Partial<ResponseInit> = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

describe("DaemonClient", () => {
  it("sends bearer token + JSON body on POST", async () => {
    const fetchFn = vi.fn().mockResolvedValue(
      jsonResponse({ session_id: "s1", worktree_ready: true, repo_local_path: "/tmp/r" }),
    );
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1234", token: "tkn" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    const r = await c.createOrUpdateSession("https://github.com/x/y/pull/1", {
      owner: "x",
      repo: "y",
      number: 1,
      title: null,
      description: null,
      author: null,
      base_branch: null,
      head_branch: null,
      files_changed: [],
    });
    expect(r.session_id).toBe("s1");
    const [url, init] = fetchFn.mock.calls[0]!;
    expect(url).toBe("http://127.0.0.1:1234/v1/sessions");
    expect((init as RequestInit).method).toBe("POST");
    const headers = (init as RequestInit).headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer tkn");
    expect(headers["Content-Type"]).toBe("application/json");
  });

  it("normalizes 401 into a DaemonError with unauthorized category", async () => {
    const fetchFn = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "unauthorized", message: "no token" }), {
        status: 401,
      }),
    );
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "x" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    await expect(c.getHealth()).rejects.toMatchObject({
      status: 401,
      category: "unauthorized",
    });
  });

  it("treats 403 as origin_rejected fallback when no envelope present", async () => {
    const fetchFn = vi.fn().mockResolvedValue(new Response("forbidden", { status: 403 }));
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "x" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    await expect(c.getVerbs()).rejects.toBeInstanceOf(DaemonError);
    await expect(c.getVerbs()).rejects.toMatchObject({
      status: 403,
      category: "origin_rejected",
    });
  });

  it("surfaces CORS/network failure as transport error", async () => {
    const fetchFn = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "x" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    await expect(c.getHealth()).rejects.toMatchObject({
      status: 0,
      category: "transport",
    });
  });

  it("returns undefined on 204 from deleteSession", async () => {
    const fetchFn = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "x" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    await expect(c.deleteSession("abc")).resolves.toBeUndefined();
  });
});
