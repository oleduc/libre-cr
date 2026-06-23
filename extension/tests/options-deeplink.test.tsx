import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

import { Options, parsePairDeepLink } from "../entrypoints/options/Options";
import { getDaemonAuth } from "../utils/daemon/storage";

describe("Options — deep-link pairing (I15)", () => {
  afterEach(() => {
    cleanup();
    // Reset hash so tests don't bleed.
    try {
      window.location.hash = "";
    } catch {
      // ignore
    }
    vi.restoreAllMocks();
  });

  it("parsePairDeepLink extracts endpoint + code + auto flag", () => {
    const r = parsePairDeepLink(
      "#pair?endpoint=http://127.0.0.1:8765&code=ABCDEF&auto=1",
    );
    expect(r).not.toBeNull();
    expect(r!.endpoint).toBe("http://127.0.0.1:8765");
    expect(r!.code).toBe("ABCDEF");
    expect(r!.auto).toBe(true);
  });

  it("returns null for non-pair hashes", () => {
    expect(parsePairDeepLink("#settings")).toBeNull();
    expect(parsePairDeepLink("")).toBeNull();
  });

  it("auto-pairs when location.hash carries auto=1 and credentials", async () => {
    window.location.hash = "#pair?endpoint=http://127.0.0.1:8765&code=XYZ&auto=1";

    const fetchSpy = vi.fn().mockImplementation((url: string, init: RequestInit) => {
      if (url.endsWith("/v1/pair")) {
        const body = JSON.parse(init.body as string) as { code: string };
        expect(body.code).toBe("XYZ");
        return Promise.resolve(
          new Response(
            JSON.stringify({ token: "tok123", extension_origin: "chrome-extension://abc" }),
            { status: 200, headers: { "Content-Type": "application/json" } },
          ),
        );
      }
      return Promise.resolve(new Response("{}", { status: 200 }));
    });
    vi.stubGlobal("fetch", fetchSpy);

    render(<Options />);
    // The pairing call should fire without any user interaction.
    await waitFor(() => {
      expect(fetchSpy).toHaveBeenCalled();
    });
    await waitFor(async () => {
      const auth = await getDaemonAuth();
      expect(auth?.token).toBe("tok123");
    });
    expect(await screen.findByText(/Paired successfully/)).toBeTruthy();
  });
});
