import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { DaemonClient } from "../utils/daemon/client";
import { ExportModal, buildExportBody } from "../components/ExportModal";

function jsonResponse(body: unknown, init: Partial<ResponseInit> = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

describe("ExportModal", () => {
  afterEach(() => cleanup());

  it("buildExportBody encodes content and severity_min correctly", () => {
    expect(buildExportBody("notes_only", "markdown", "any")).toEqual({
      format: "markdown",
      filter: { include_thinking: false, severity_min: null },
    });
    expect(
      buildExportBody("notes_plus_context", "github_review", "warning"),
    ).toEqual({
      format: "github_review",
      filter: { include_thinking: true, severity_min: "warning" },
    });
  });

  it("POSTs to /v1/sessions/:id/export with the chosen body and copies markdown", async () => {
    const fetchFn = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      const url = _url as string;
      if (url.endsWith("/export")) {
        // Verify the body shape sent matches what the radios were set to.
        const body = JSON.parse(init.body as string);
        expect(body.format).toBe("markdown");
        expect(body.filter.severity_min).toBe("suggestion");
        expect(body.filter.include_thinking).toBe(true);
        return Promise.resolve(
          jsonResponse({ content: "# Review draft\n\nhello" }),
        );
      }
      return Promise.resolve(new Response("{}", { status: 200 }));
    });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(globalThis.navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });

    const client = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: fetchFn as unknown as typeof fetch },
    );

    render(<ExportModal client={client} sessionId="sess1" onClose={() => {}} />);

    // Pick "notes + context" + min severity = suggestion.
    fireEvent.click(screen.getByLabelText(/Notes \+ investigation context/));
    fireEvent.change(screen.getByTestId("severity-min"), {
      target: { value: "suggestion" },
    });
    fireEvent.click(screen.getByTestId("export-go"));

    // Wait for toast.
    const toast = await screen.findByTestId("export-toast");
    expect(toast.textContent).toMatch(/Copied to clipboard|Generated/);
    expect(writeText).toHaveBeenCalledWith("# Review draft\n\nhello");
  });

  it("renders GitHub structured preview when format is github_review", async () => {
    const fetchFn = vi.fn().mockResolvedValue(
      jsonResponse({
        content: "**Warning**\n\n- noted",
        structure: {
          body: "**Warning**\n\n- noted",
          event: "REQUEST_CHANGES",
          comments: [{ path: "src/a.ts", line: 5, body: "fix this" }],
        },
      }),
    );
    const client = new DaemonClient(
      { endpoint: "http://127.0.0.1:1", token: "t" },
      { fetch: fetchFn as unknown as typeof fetch },
    );
    render(<ExportModal client={client} sessionId="sess1" onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText(/GitHub review/));
    fireEvent.click(screen.getByTestId("export-go"));
    expect(await screen.findByText(/REQUEST_CHANGES/)).toBeTruthy();
    expect(screen.getByText(/fix this/)).toBeTruthy();
  });
});
