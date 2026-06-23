import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { QaPanel } from "../components/QaPanel";

function mockClientWithVerbs(): DaemonClient {
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

describe("QaPanel — Phase 6 surface", () => {
  afterEach(() => cleanup());

  it("shows the empty-state onboarding tooltip on first paint", async () => {
    const client = mockClientWithVerbs();
    render(
      <QaPanel
        client={client}
        sessionId="s"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    expect(await screen.findByTestId("empty-state")).toBeTruthy();
    expect(screen.getByTestId("freeform-hint")).toBeTruthy();
  });

  it("renders the diff-changed banner when prDiffChanged is true and dismisses it", async () => {
    const client = mockClientWithVerbs();
    render(
      <QaPanel
        client={client}
        sessionId="s_diff"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
        prDiffChanged
        headSha="newsha"
      />,
    );
    const banner = await screen.findByTestId("diff-changed-banner");
    expect(banner.textContent).toMatch(/PR diff changed/);
    fireEvent.click(screen.getByTestId("diff-changed-dismiss"));
    await waitFor(() => {
      expect(screen.queryByTestId("diff-changed-banner")).toBeNull();
    });
  });

  it("opens the export modal from the title-bar button", async () => {
    const client = mockClientWithVerbs();
    render(
      <QaPanel
        client={client}
        sessionId="s"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    fireEvent.click(await screen.findByTestId("open-export"));
    expect(screen.getByTestId("export-modal")).toBeTruthy();
  });
});
