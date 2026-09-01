import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";

import { Markdown, renderMarkdown } from "../components/Markdown";

describe("Markdown", () => {
  it("renders tables, code and emphasis", () => {
    const { container } = render(
      <Markdown text={"| a | b |\n|---|---|\n| 1 | 2 |\n\n**bold** and `Vec<String>`\n\n```rust\nlet x: Vec<String> = vec![];\n```"} />,
    );
    expect(container.querySelector("table")).toBeTruthy();
    expect(container.querySelector("strong")!.textContent).toBe("bold");
    // Generics inside code survive verbatim.
    expect(container.querySelector("code")!.textContent).toBe("Vec<String>");
    expect(container.querySelector("pre code")!.textContent).toContain("Vec<String>");
  });

  it("strips scripts, event handlers and non-http links", () => {
    const html = renderMarkdown(
      'x <script>alert(1)</script> <img src=x onerror=alert(1)> [ok](https://a.b) [bad](javascript:alert(1))',
    );
    expect(html).not.toContain("<script");
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("javascript:");
    expect(html).toContain('href="https://a.b"');
    expect(html).toContain('rel="noopener noreferrer"');
  });
});
