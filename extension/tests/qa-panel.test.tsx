import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

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

// Fake WS transport: captures the AskInit each ask sends and lets tests
// answer with server frames.
interface FakeWs {
  url: string;
  sent: string[];
  onopen?: () => void;
  onmessage?: (ev: { data: string }) => void;
  onerror?: () => void;
  onclose?: (ev: { reason?: string }) => void;
  send(s: string): void;
  close(): void;
}
const fakeSockets = vi.hoisted(() => [] as FakeWs[]);
vi.mock("../utils/daemon/proxy", () => ({
  daemonWsFactory: () => (url: string) => {
    const ws: FakeWs = {
      url,
      sent: [],
      send(s: string) {
        this.sent.push(s);
      },
      close() {},
    };
    fakeSockets.push(ws);
    return ws;
  },
}));

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

describe("QaPanel context turns", () => {
  afterEach(() => cleanup());

  async function askOnce(text: string): Promise<FakeWs> {
    const before = fakeSockets.length;
    const box = screen.getByPlaceholderText(/Ask a question/);
    fireEvent.change(box, { target: { value: text } });
    fireEvent.click(screen.getByText(/Ask ▶/));
    await waitFor(() => expect(fakeSockets.length).toBe(before + 1));
    const ws = fakeSockets[fakeSockets.length - 1]!;
    ws.onopen?.();
    await waitFor(() => expect(ws.sent.length).toBe(1));
    return ws;
  }

  it("sends expanded turns' daemon ids and records the new turn's id", async () => {
    fakeSockets.length = 0;
    const client = mockClient();
    render(
      <QaPanel
        client={client}
        sessionId="s"
        prSlug="x/y#1"
        selection={null}
        onClearSelection={() => {}}
        onClose={() => {}}
        initialTurns={[
          {
            kind: "qa",
            id: "t_hist",
            daemonTurnId: "t_hist",
            question: "old q",
            answer: "old a",
            collapsed: false,
          },
        ]}
      />,
    );
    await screen.findByText("Explain"); // verbs loaded → panel settled

    // First ask: the restored turn is expanded, so its id goes on the wire.
    const ws1 = await askOnce("follow up?");
    const init1 = JSON.parse(ws1.sent[0]!);
    expect(init1.context_turn_ids).toEqual(["t_hist"]);
    ws1.onmessage?.({
      data: JSON.stringify({
        type: "done",
        turn_id: "t_new",
        usage: { input_tokens: 1, output_tokens: 1 },
      }),
    });
    await waitFor(() => expect(screen.getByText(/Ask ▶/)).toBeTruthy());

    // Second ask: the first ask collapsed t_hist; only the fresh turn — now
    // stamped with the daemon's id from `done` — is expanded.
    const ws2 = await askOnce("and another?");
    const init2 = JSON.parse(ws2.sent[0]!);
    expect(init2.context_turn_ids).toEqual(["t_new"]);
  });
});
