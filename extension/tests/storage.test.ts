import { describe, expect, it } from "vitest";

import {
  clearDaemonAuth,
  getDaemonAuth,
  getPanelPosition,
  setDaemonAuth,
  setPanelPosition,
} from "../utils/daemon/storage";

describe("storage wrappers", () => {
  it("round-trips daemon auth", async () => {
    expect(await getDaemonAuth()).toBeNull();
    await setDaemonAuth({
      endpoint: "http://127.0.0.1:9",
      token: "tk",
      extensionOrigin: "chrome-extension://x",
    });
    const a = await getDaemonAuth();
    expect(a).toEqual({
      endpoint: "http://127.0.0.1:9",
      token: "tk",
      extensionOrigin: "chrome-extension://x",
    });
    await clearDaemonAuth();
    expect(await getDaemonAuth()).toBeNull();
  });

  it("persists panel position per PR url", async () => {
    await setPanelPosition("https://github.com/x/y/pull/1", { x: 1, y: 2, width: 3, height: 4 });
    await setPanelPosition("https://github.com/x/y/pull/2", { x: 10, y: 20, width: 30, height: 40 });
    expect(await getPanelPosition("https://github.com/x/y/pull/1")).toEqual({
      x: 1, y: 2, width: 3, height: 4,
    });
    expect(await getPanelPosition("https://github.com/x/y/pull/2")).toEqual({
      x: 10, y: 20, width: 30, height: 40,
    });
    expect(await getPanelPosition("nope")).toBeUndefined();
  });
});
