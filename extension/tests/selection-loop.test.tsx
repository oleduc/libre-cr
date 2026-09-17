import { describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { SelectionLayer } from "../components/SelectionLayer";
import type { Selection } from "../utils/selection";

// The crash this pins: a caller that builds its handler inline — the normal
// way — made SelectionLayer's effect re-run on every render. Re-attaching
// re-reads the URL hash and re-emits the selection, which re-renders, which
// re-attaches. The loop allocated until Chrome killed the tab ("Aw, Snap").
describe("SelectionLayer does not re-attach on every render", () => {
  it("keeps one set of document listeners across renders with a new callback", () => {
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    const noop = (_sel: Selection | null) => {};

    const { rerender } = render(<SelectionLayer onSelect={() => noop(null)} />);
    const afterFirst = add.mock.calls.length;
    expect(afterFirst).toBeGreaterThan(0);

    // Three renders, each with a freshly built handler.
    for (let i = 0; i < 3; i += 1) {
      rerender(<SelectionLayer onSelect={() => noop(null)} />);
    }

    expect(add.mock.calls.length).toBe(afterFirst);
    expect(remove.mock.calls.length).toBe(0);
    add.mockRestore();
    remove.mockRestore();
    cleanup();
  });

  it("still calls the latest handler, not the one captured at mount", () => {
    const first = vi.fn();
    const second = vi.fn();
    document.body.innerHTML =
      '<div class="file" data-tagsearch-path="src/a.ts"><table><tr>' +
      '<td class="blob-num" data-line-number="3"></td>' +
      '<td class="blob-code">const x = 1;</td></tr></table></div>';

    const { rerender } = render(<SelectionLayer onSelect={first} />);
    rerender(<SelectionLayer onSelect={second} />);

    document.querySelector("td.blob-code")!.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "line", file: "src/a.ts", line: 3 }),
    );
    cleanup();
  });
});
