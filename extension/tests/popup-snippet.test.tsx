import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { renderSnippet } from "../entrypoints/popup/Popup";

describe("popup snippet rendering (C5)", () => {
  afterEach(() => cleanup());

  it("wraps `[…]` FTS5 markers in <mark> and renders text safely", () => {
    const out = renderSnippet("foo [bcrypt] cost [12]");
    const { container } = render(<div>{out}</div>);
    const marks = container.querySelectorAll("mark");
    expect(marks.length).toBe(2);
    expect(marks[0]!.textContent).toBe("bcrypt");
    expect(marks[1]!.textContent).toBe("12");
    expect(container.textContent).toBe("foo bcrypt cost 12");
  });

  it("does not execute embedded HTML/script tags — daemon snippet is not parsed as HTML", () => {
    const malicious = "[<script>window.x=1</script>] tail";
    const { container } = render(<div data-testid="root">{renderSnippet(malicious)}</div>);
    // No script element should have been created from the snippet.
    expect(container.querySelector("script")).toBeNull();
    // The literal `<script>` text should be present as escaped text inside <mark>.
    const mark = container.querySelector("mark");
    expect(mark).not.toBeNull();
    expect(mark!.textContent).toContain("<script>");
  });

  it("returns a plain string when no markers are present", () => {
    expect(renderSnippet("nothing to mark")).toBe("nothing to mark");
  });
});
