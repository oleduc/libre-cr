import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

// A PR whose head moved since the session was created: the daemon answers
// `worktree_ready: false` with `pending_action: "worktree_updating"`, and the
// panel must say the branch is being updated rather than repeating the
// first-clone copy — the wait is the same, what to expect is not.
vi.mock("../utils/daemon/storage", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../utils/daemon/storage")>()),
  getDaemonAuth: async () => ({ endpoint: "http://127.0.0.1:8765", token: "t" }),
}));
vi.mock("../utils/daemon/proxy", () => ({
  daemonFetch: () => async (url: string) => {
    const json = (body: unknown) =>
      new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });
    if (url.endsWith("/v1/sessions")) {
      return json({
        session_id: "s_1",
        worktree_ready: false,
        pending_action: "worktree_updating",
        pr_diff_changed: true,
        head_sha: "abc",
      });
    }
    // Never becomes ready: the banner is what this test is about.
    return json({ session_id: "s_1", worktree_ready: false, turns: [] });
  },
}));

import { ContentApp } from "../components/ContentApp";

describe("a worktree behind the PR head", () => {
  it("says the branch is updating, not that it is cloning", async () => {
    render(
      <ContentApp
        prUrl="https://github.com/o/r/pull/1"
        styleEl={document.createElement("style")}
      />,
    );
    await waitFor(() => expect(screen.getByTestId("worktree-updating")).toBeTruthy());
    expect(screen.getByText(/updating the branch/i)).toBeTruthy();
    expect(screen.queryByText(/first visit to a repo clones it/i)).toBeNull();
  });
});
