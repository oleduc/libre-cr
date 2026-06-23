import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { QaPanel } from "../components/QaPanel";

const VERBS = [
  {
    id: "find_callers",
    label: "Find callers",
    required_selection: "symbol" as const,
    description: "",
    suggested_tools: [],
  },
  {
    id: "explain",
    label: "Explain",
    required_selection: "file" as const,
    description: "",
    suggested_tools: [],
  },
];

function mockClient(): DaemonClient {
  const fetchFn = vi.fn().mockImplementation((url: string) => {
    if (url.endsWith("/v1/verbs")) {
      return Promise.resolve(
        new Response(JSON.stringify(VERBS), {
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

describe("QaPanel", () => {
  afterEach(() => cleanup());

  it("renders verbs from /v1/verbs and disables ones the selection doesn't satisfy", async () => {
    const client = mockClient();
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
    // Verbs are async; wait for them.
    const findCallers = await screen.findByText("Find callers");
    const explain = await screen.findByText("Explain");
    expect((findCallers as HTMLButtonElement).disabled).toBe(true);
    expect((explain as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows selection label when a selection is present", async () => {
    const client = mockClient();
    render(
      <QaPanel
        client={client}
        sessionId="s"
        prSlug="x/y#1"
        selection={{ kind: "line", file: "src/a.ts", line: 7 }}
        onClearSelection={() => {}}
        onClose={() => {}}
      />,
    );
    expect(await screen.findByText(/Selection: src\/a\.ts:7/)).toBeTruthy();
  });
});
