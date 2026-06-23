import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { QaPanel } from "../components/QaPanel";

function mockClient(): DaemonClient {
  const fetchFn = vi.fn().mockImplementation((url: string) => {
    if (url.endsWith("/v1/verbs")) {
      return Promise.resolve(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    }
    return Promise.resolve(new Response("{}", { status: 200 }));
  });
  return new DaemonClient(
    { endpoint: "http://127.0.0.1:1", token: "t" },
    { fetch: fetchFn as unknown as typeof fetch },
  );
}

describe("QaPanel — presentation cleanup on unmount (C6)", () => {
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("removes data-libre-cr-effect-id from the DOM when the panel unmounts", () => {
    // Simulate a previously applied highlight + annotation living in the
    // GitHub page DOM (outside the panel's shadow root).
    const row = document.createElement("tr");
    row.setAttribute("data-libre-cr-effect-id", "e1");
    row.setAttribute("data-libre-cr-tag", "highlight");
    row.classList.add("libre-cr-effect", "libre-cr-hl-yellow");
    document.body.appendChild(row);

    const ann = document.createElement("tr");
    ann.setAttribute("data-libre-cr-effect-id", "e2");
    ann.setAttribute("data-libre-cr-tag", "annotation");
    document.body.appendChild(ann);

    const { unmount } = render(
      <QaPanel
        client={mockClient()}
        sessionId="s"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );

    expect(document.querySelectorAll("[data-libre-cr-effect-id]").length).toBe(2);
    unmount();

    // Highlights have their attributes stripped; annotation rows are removed.
    expect(document.querySelectorAll("[data-libre-cr-effect-id]").length).toBe(0);
  });
});
