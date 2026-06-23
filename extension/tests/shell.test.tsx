// I13: Shell geometry — persisted height is applied, drags are clamped to
// the viewport on mouseup, and window listeners are removed if the tree
// unmounts mid-drag.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { Shell } from "../components/Shell";
import {
  __resetMemoryStore,
  getPanelPosition,
  setPanelPosition,
} from "../utils/daemon/storage";

const PR = "https://github.com/x/y/pull/1";

function renderShell() {
  return render(
    <Shell prUrl={PR}>
      <div className="libre-cr-titlebar">drag me</div>
      <div>body</div>
    </Shell>,
  );
}

describe("Shell — geometry persistence and drag (I13)", () => {
  beforeEach(() => __resetMemoryStore());
  afterEach(() => cleanup());

  it("applies the persisted height (not just top/left/width)", async () => {
    await setPanelPosition(PR, { x: 10, y: 20, width: 300, height: 444 });
    const { container } = renderShell();
    const shell = container.querySelector<HTMLElement>(".libre-cr-shell")!;
    await waitFor(() => {
      expect(shell.style.top).toBe("20px");
      expect(shell.style.left).toBe("10px");
      expect(shell.style.width).toBe("300px");
      expect(shell.style.height).toBe("444px");
    });
  });

  it("clamps a drag past the viewport edge on mouseup and persists the clamped position", async () => {
    const { container } = renderShell();
    const titlebar = container.querySelector<HTMLElement>(".libre-cr-titlebar")!;
    const shell = container.querySelector<HTMLElement>(".libre-cr-shell")!;

    fireEvent.mouseDown(titlebar, { clientX: 5, clientY: 5 });
    // Drag way off-screen (jsdom viewport is 1024x768).
    fireEvent.mouseMove(window, { clientX: 5000, clientY: 5000 });
    fireEvent.mouseUp(window);

    const maxX = window.innerWidth - 40;
    const maxY = window.innerHeight - 40;
    await waitFor(() => {
      expect(shell.style.left).toBe(`${maxX}px`);
      expect(shell.style.top).toBe(`${maxY}px`);
    });
    const persisted = await getPanelPosition(PR);
    expect(persisted).toMatchObject({ x: maxX, y: maxY });
  });

  it("removes window listeners when unmounted mid-drag", () => {
    const removeSpy = vi.spyOn(window, "removeEventListener");
    const { container, unmount } = renderShell();
    const titlebar = container.querySelector<HTMLElement>(".libre-cr-titlebar")!;

    fireEvent.mouseDown(titlebar, { clientX: 5, clientY: 5 });
    fireEvent.mouseMove(window, { clientX: 50, clientY: 50 });
    unmount();

    const removed = removeSpy.mock.calls.map((c) => c[0]);
    expect(removed).toContain("mousemove");
    expect(removed).toContain("mouseup");
    removeSpy.mockRestore();

    // And a stray mousemove after unmount must not throw / mutate anything.
    fireEvent.mouseMove(window, { clientX: 60, clientY: 60 });
  });
});
