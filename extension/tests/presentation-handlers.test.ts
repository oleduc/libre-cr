import { beforeEach, describe, expect, it } from "vitest";

import {
  annotateLine,
  clearPresentation,
  highlightLines,
  isSafeUrl,
  makeContext,
  openLink,
  scrollTo,
} from "../utils/presentation/handlers";

const FIXTURE = `
<div class="file" data-tagsearch-path="src/auth.ts">
  <table>
    <tr><td class="blob-num" data-line-number="10"></td><td class="blob-code">a</td></tr>
    <tr><td class="blob-num" data-line-number="11"></td><td class="blob-code">b</td></tr>
    <tr><td class="blob-num" data-line-number="12"></td><td class="blob-code">c</td></tr>
  </table>
</div>
`;

describe("presentation handlers", () => {
  beforeEach(() => {
    document.body.innerHTML = FIXTURE;
  });

  it("highlight_lines tags rows with effect-id and color class", () => {
    const ctx = makeContext();
    const r = highlightLines(ctx, { file: "src/auth.ts", start_line: 10, end_line: 11, color: "red" });
    expect(r.ok).toBe(true);
    const tagged = document.querySelectorAll("[data-libre-cr-effect-id]");
    expect(tagged.length).toBe(2);
    expect(document.querySelector(".libre-cr-hl-red")).not.toBeNull();
  });

  it("highlight_lines returns file_not_in_view when no row found", () => {
    const ctx = makeContext();
    const r = highlightLines(ctx, { file: "missing.ts", start_line: 1 });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe("file_not_in_view");
  });

  it("annotate_line injects a row via textContent (no innerHTML)", () => {
    const ctx = makeContext();
    const r = annotateLine(ctx, {
      file: "src/auth.ts",
      line: 11,
      summary: "<script>nope</script>",
      detail: "be defensive",
      severity: "warning",
    });
    expect(r.ok).toBe(true);
    const annotation = document.querySelector("[data-libre-cr-tag='annotation']")!;
    expect(annotation).not.toBeNull();
    // The HTML should be escaped — no actual <script> element.
    expect(annotation.querySelector("script")).toBeNull();
    expect(annotation.textContent).toContain("<script>nope</script>");
  });

  it("scroll_to flashes a row", () => {
    const ctx = makeContext();
    const r = scrollTo(ctx, { file: "src/auth.ts", line: 12 });
    expect(r.ok).toBe(true);
    expect(document.querySelector(".libre-cr-flash")).not.toBeNull();
  });

  it("open_link rejects javascript:, data:, and file: URLs", () => {
    const ctx = makeContext();
    for (const bad of [
      "javascript:alert(1)",
      "data:text/html,xxx",
      "file:///etc/passwd",
      "//evil.com",
    ]) {
      const r = openLink(ctx, { url: bad });
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.error).toBe("url_rejected");
    }
  });

  it("open_link rejects panel target by default", () => {
    const ctx = makeContext();
    const r = openLink(ctx, { url: "https://github.com/foo", target: "panel" });
    expect(r.ok).toBe(false);
  });

  it("open_link accepts https + relative paths", () => {
    const ctx = makeContext();
    expect(isSafeUrl("https://github.com/x")).toBe(true);
    expect(isSafeUrl("/foo/bar")).toBe(true);
    const r = openLink(ctx, { url: "https://github.com/foo" });
    expect(r.ok).toBe(true);
  });

  it("clear_presentation removes annotations and clears highlight markers", () => {
    const ctx = makeContext();
    highlightLines(ctx, { file: "src/auth.ts", start_line: 10, end_line: 10 });
    annotateLine(ctx, { file: "src/auth.ts", line: 11, summary: "x" });
    expect(document.querySelectorAll("[data-libre-cr-effect-id]").length).toBe(2);
    clearPresentation(ctx, "all");
    expect(document.querySelectorAll("[data-libre-cr-effect-id]").length).toBe(0);
  });
});
