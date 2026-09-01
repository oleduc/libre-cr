import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";

import { fragmentToMarkdown, Markdown, renderMarkdown } from "../components/Markdown";

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

function fragOf(md: string): DocumentFragment {
  const div = document.createElement("div");
  div.innerHTML = renderMarkdown(md);
  document.body.appendChild(div);
  const r = document.createRange();
  r.selectNodeContents(div);
  const frag = r.cloneContents();
  div.remove();
  return frag;
}

describe("copy as markdown", () => {
  it("round-trips emphasis, code, links and headings", () => {
    const md = fragmentToMarkdown(fragOf("## Title\n\n**bold** *em* `code` [x](https://a.b)"));
    expect(md).toContain("## Title");
    expect(md).toContain("**bold**");
    expect(md).toContain("*em*");
    expect(md).toContain("`code`");
    expect(md).toContain("[x](https://a.b)");
  });

  it("round-trips lists, fences and tables", () => {
    const src = "- one\n- two\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |";
    const md = fragmentToMarkdown(fragOf(src));
    expect(md).toContain("- one");
    expect(md).toContain("- two");
    expect(md).toContain("```rust\nlet x = 1;\n```");
    expect(md).toContain("| a | b |");
    expect(md).toContain("| 1 | 2 |");
  });

  it("a partial inline selection stays plain text", () => {
    const div = document.createElement("div");
    div.innerHTML = renderMarkdown("plain **bold** tail");
    const p = div.querySelector("p")!;
    const r = document.createRange();
    r.setStart(p.firstChild!, 0);
    r.setEnd(p.firstChild!, 5);
    expect(fragmentToMarkdown(r.cloneContents())).toBe("plain");
  });

  it("blockquotes and nested lists survive", () => {
    const md = fragmentToMarkdown(fragOf("> quoted line\n\n- outer\n  - inner"));
    expect(md).toContain("> quoted line");
    expect(md).toContain("- outer");
    expect(md).toContain("  - inner");
  });
});
