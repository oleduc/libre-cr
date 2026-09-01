// I14: the SelectionLayer's capture click listener must only react to clicks
// on diff-table cells inside a `[data-tagsearch-path]` container — never to
// links/buttons inside a diff row or to clicks elsewhere on the page.

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { SelectionLayer } from "../components/SelectionLayer";
import type { Selection } from "../utils/selection";

function buildDiffFixture(): HTMLElement {
  const container = document.createElement("div");
  container.setAttribute("data-tagsearch-path", "src/a.ts");
  container.innerHTML =
    "<table><tbody><tr>" +
    '<td class="blob-num" data-line-number="3"></td>' +
    '<td class="blob-code">foo<a href="https://example.com">link</a></td>' +
    "</tr></tbody></table>";
  document.body.appendChild(container);
  return container;
}

function buildReactUiFixture(): HTMLElement {
  const table = document.createElement("table");
  table.setAttribute("aria-label", "Diff for: src/b.rs");
  table.innerHTML =
    '<tbody><tr class="diff-line-row">' +
    '<td class="focusable-grid-cell new-diff-line-number" data-diff-side="left" data-line-number="7">7</td>' +
    '<td class="focusable-grid-cell new-diff-line-number" data-diff-side="right" data-line-number="8">8</td>' +
    '<td class="diff-text-cell focusable-grid-cell" data-diff-side="right" data-line-number="8">let x = 1;</td>' +
    "</tr></tbody>";
  document.body.appendChild(table);
  return table;
}

describe("SelectionLayer — GitHub React 'changes' UI", () => {
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("selects a line from the new DOM's cells", () => {
    const table = buildReactUiFixture();
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);
    fireEvent.click(table.querySelector("td.diff-text-cell")!);
    expect(onSelect).toHaveBeenCalledWith({ kind: "line", file: "src/b.rs", line: 8, text: "let x = 1;" });
  });
});

describe("SelectionLayer — symbol pick uses the clicked cell", () => {
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("cmd-click on the right code cell of a replacement row reads right-side text", () => {
    const table = document.createElement("table");
    table.setAttribute("aria-label", "Diff for: src/c.rs");
    table.innerHTML =
      '<tbody><tr class="diff-line-row">' +
      '<td class="focusable-grid-cell new-diff-line-number" data-diff-side="left" data-line-number="4">4</td>' +
      '<td class="diff-text-cell focusable-grid-cell" data-diff-side="left" data-line-number="4">old_name</td>' +
      '<td class="focusable-grid-cell new-diff-line-number" data-diff-side="right" data-line-number="5">5</td>' +
      '<td class="diff-text-cell focusable-grid-cell" data-diff-side="right" data-line-number="5">new_name</td>' +
      "</tbody></tr>";
    document.body.appendChild(table);
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);
    const right = table.querySelector('td.diff-text-cell[data-diff-side="right"]')!;
    fireEvent.click(right, { metaKey: true });
    expect(onSelect).toHaveBeenCalledTimes(1);
    const sel = onSelect.mock.calls[0]![0]!;
    expect(sel.kind).toBe("symbol");
    expect((sel as { identifier: string }).identifier).toBe("new_name");
  });
});

describe("SelectionLayer — click scoping (I14)", () => {
  afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
  });

  it("plain click on a diff cell selects the line", () => {
    const container = buildDiffFixture();
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);

    fireEvent.click(container.querySelector("td.blob-num")!);
    expect(onSelect).toHaveBeenCalledWith({ kind: "line", file: "src/a.ts", line: 3, text: "foolink" });
  });

  it("cmd-click on a diff code cell picks a symbol", () => {
    const container = buildDiffFixture();
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);

    fireEvent.click(container.querySelector("td.blob-code")!, {
      metaKey: true,
      clientX: 10,
    });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0][0]).toMatchObject({
      kind: "symbol",
      file: "src/a.ts",
      line: 3,
    });
  });

  it("ignores clicks on links inside a diff row", () => {
    const container = buildDiffFixture();
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);

    fireEvent.click(container.querySelector("a")!);
    fireEvent.click(container.querySelector("a")!, { metaKey: true });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("ignores clicks outside any diff container", () => {
    buildDiffFixture();
    const outside = document.createElement("div");
    outside.innerHTML = "<table><tbody><tr><td>not a diff</td></tr></tbody></table>";
    document.body.appendChild(outside);
    const onSelect = vi.fn<(s: Selection | null) => void>();
    render(<SelectionLayer onSelect={onSelect} />);

    fireEvent.click(outside.querySelector("td")!);
    fireEvent.click(document.body);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
