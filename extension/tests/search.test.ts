import { describe, expect, it, vi } from "vitest";

import { DaemonClient } from "../utils/daemon/client";

describe("DaemonClient.search", () => {
  it("builds the /v1/search URL with q + limit and parses results", async () => {
    const fetchFn = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          results: [
            {
              session_id: "s1",
              pr_url: "https://github.com/x/y/pull/1",
              turn_id: "t1",
              snippet: "[bcrypt] cost",
              score: 1.2,
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );
    const c = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    const r = await c.search("bcrypt cost", 10);
    const [url] = fetchFn.mock.calls[0]!;
    expect(url).toBe("http://127.0.0.1:1/v1/search?q=bcrypt%20cost&limit=10");
    expect(r.results).toHaveLength(1);
    expect(r.results[0]!.session_id).toBe("s1");
  });
});
