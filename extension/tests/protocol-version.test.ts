// Protocol-version soft check: `getHealth()` compares the daemon's
// `protocol_version` to ours and records `ui.protocol_mismatch` (warning
// only — never a hard failure). Missing field = older daemon = compatible.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DaemonClient } from "../utils/daemon/client";
import { PROTOCOL_VERSION } from "../utils/daemon/frames";
import { recordProtocolCheck } from "../utils/daemon/protocol";
import { __resetMemoryStore, getKey, setKey } from "../utils/daemon/storage";

function clientWithHealth(body: Record<string, unknown>): DaemonClient {
  const fetchFn = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }),
  );
  return new DaemonClient(
    { endpoint: "http://127.0.0.1:1", token: "t" },
    { fetch: fetchFn as unknown as typeof fetch },
  );
}

describe("protocol-version soft check", () => {
  beforeEach(() => __resetMemoryStore());
  afterEach(() => vi.restoreAllMocks());

  it("mismatched protocol_version sets ui.protocol_mismatch and warns", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const c = clientWithHealth({
      ok: true,
      version: "0.1.0",
      protocol_version: PROTOCOL_VERSION + 1,
    });
    const health = await c.getHealth();
    expect(health.ok).toBe(true);

    const rec = await getKey("ui.protocol_mismatch");
    expect(rec).toMatchObject({
      daemon: PROTOCOL_VERSION + 1,
      extension: PROTOCOL_VERSION,
    });
    expect(warn).toHaveBeenCalledOnce();
  });

  it("matching protocol_version does not warn and clears a stale record", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await setKey("ui.protocol_mismatch", { at: 1, daemon: 99, extension: PROTOCOL_VERSION });
    const c = clientWithHealth({
      ok: true,
      version: "0.1.0",
      protocol_version: PROTOCOL_VERSION,
    });
    await c.getHealth();
    expect(await getKey("ui.protocol_mismatch")).toBeUndefined();
    expect(warn).not.toHaveBeenCalled();
  });

  it("absent protocol_version (older daemon) does not warn", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const c = clientWithHealth({ ok: true, version: "0.1.0" });
    await c.getHealth();
    expect(await getKey("ui.protocol_mismatch")).toBeUndefined();
    expect(warn).not.toHaveBeenCalled();
  });

  it("recordProtocolCheck returns false only on a real mismatch", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(await recordProtocolCheck({})).toBe(true);
    expect(await recordProtocolCheck({ protocol_version: PROTOCOL_VERSION })).toBe(true);
    expect(await recordProtocolCheck({ protocol_version: PROTOCOL_VERSION + 1 })).toBe(false);
  });
});
