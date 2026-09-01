// The relayed fetch must forward a `Request` input's method, headers and
// body — not just `init`'s. A POST built as `new Request(...)` used to be
// relayed as a bare GET.

import { afterEach, describe, expect, it, vi } from "vitest";

import { FETCH_MSG, daemonFetch } from "../utils/daemon/proxy";
import type { FetchRequestMsg } from "../utils/daemon/proxy";

function installFakeRuntime() {
  const sent: FetchRequestMsg[] = [];
  const sendMessage = vi.fn().mockImplementation((msg: FetchRequestMsg) => {
    sent.push(msg);
    return Promise.resolve({ ok: true, status: 200, statusText: "OK", headers: [], body: "{}" });
  });
  (globalThis as unknown as { chrome?: unknown }).chrome = {
    runtime: { sendMessage, connect: () => ({}) },
  };
  return sent;
}

describe("relayed fetch with Request inputs", () => {
  afterEach(() => {
    delete (globalThis as unknown as { chrome?: unknown }).chrome;
  });

  it("forwards method, headers and body from a Request", async () => {
    const sent = installFakeRuntime();
    const req = new Request("http://127.0.0.1:1/v1/sessions", {
      method: "POST",
      headers: { Authorization: "Bearer t", "Content-Type": "application/json" },
      body: '{"pr_url":"x"}',
    });
    const res = await daemonFetch()(req);
    expect(res.status).toBe(200);
    expect(sent[0]).toMatchObject({
      type: FETCH_MSG,
      url: "http://127.0.0.1:1/v1/sessions",
      method: "POST",
      body: '{"pr_url":"x"}',
    });
    expect(sent[0]!.headers).toMatchObject({ authorization: "Bearer t" });
  });

  it("lets init override a Request's fields", async () => {
    const sent = installFakeRuntime();
    const req = new Request("http://127.0.0.1:1/x", { method: "POST", body: "a" });
    await daemonFetch()(req, { method: "PUT", body: "b" });
    expect(sent[0]).toMatchObject({ method: "PUT", body: "b" });
  });
});
