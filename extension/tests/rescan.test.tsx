import { describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { QaPanel } from "../components/QaPanel";
import { DaemonClient } from "../utils/daemon/client";

vi.mock("../utils/daemon/proxy", () => ({
  daemonWsFactory: () => () => ({ send() {}, close() {} }),
}));

function client(): DaemonClient {
  const fetchFn = vi.fn().mockResolvedValue(
    new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } }),
  );
  return new DaemonClient(
    { endpoint: "http://127.0.0.1:1", token: "t" },
    { fetch: fetchFn as unknown as typeof fetch },
  );
}

// A scrape that read the page too early — or read a view whose markup we do
// not support — leaves the session without a base branch, and the only way to
// try again used to be reloading the tab and losing the conversation.
describe("re-scan", () => {
  const base = {
    client: client(),
    sessionId: "s1",
    prSlug: "o/r#1",
    selection: null,
    onClearSelection: () => {},
    onClose: () => {},
  };

  it("offers re-scan from the warning banner and from the footer", () => {
    const onRescan = vi.fn();
    render(
      <QaPanel
        {...base}
        warnings={["missing base/head — selectors may need refresh"]}
        onRescan={onRescan}
      />,
    );

    fireEvent.click(screen.getByTestId("rescan"));
    fireEvent.click(screen.getByTestId("rescan-footer"));
    expect(onRescan).toHaveBeenCalledTimes(2);
    cleanup();
  });

  it("says it is working and refuses a second run while one is in flight", () => {
    const onRescan = vi.fn();
    render(<QaPanel {...base} warnings={["w"]} onRescan={onRescan} rescanning />);

    const button = screen.getByTestId("rescan") as HTMLButtonElement;
    expect(button.textContent).toContain("Re-scanning");
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(onRescan).not.toHaveBeenCalled();
    cleanup();
  });

  it("stays out of the way when the host offers no re-scan", () => {
    render(<QaPanel {...base} warnings={["w"]} />);
    expect(screen.queryByTestId("rescan")).toBeNull();
    expect(screen.queryByTestId("rescan-footer")).toBeNull();
    cleanup();
  });
});
