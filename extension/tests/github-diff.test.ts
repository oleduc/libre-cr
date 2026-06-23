import { describe, expect, it, beforeEach } from "vitest";

import { enumerateDiffLines, findRow, hitTestLine, pickIdentifier } from "../utils/github/diff";

const DIFF_FIXTURE = `
<div class="file" data-tagsearch-path="src/auth.ts">
  <table>
    <tr>
      <td class="blob-num" data-line-number="10"></td>
      <td class="blob-num" data-line-number="10"></td>
      <td class="blob-code">function login(user) {}</td>
    </tr>
    <tr>
      <td class="blob-num" data-line-number="11"></td>
      <td class="blob-num" data-line-number="11"></td>
      <td class="blob-code">return hashPassword(user.password);</td>
    </tr>
  </table>
</div>
`;

describe("diff utilities", () => {
  beforeEach(() => {
    document.body.innerHTML = DIFF_FIXTURE;
  });

  it("enumerates lines", () => {
    const list = enumerateDiffLines();
    expect(list).toHaveLength(2);
    expect(list[0]?.file).toBe("src/auth.ts");
    expect(list[0]?.line).toBe(10);
  });

  it("hit-tests a click on a code cell", () => {
    const td = document.querySelector(".blob-code")!;
    const hit = hitTestLine(td);
    expect(hit?.file).toBe("src/auth.ts");
    expect(hit?.line).toBe(10);
  });

  it("findRow returns the right tr", () => {
    const row = findRow("src/auth.ts", 11);
    expect(row).not.toBeNull();
    expect(row?.textContent).toContain("hashPassword");
  });
});

describe("pickIdentifier", () => {
  it("picks the identifier under a cursor", () => {
    expect(pickIdentifier("return hashPassword(user.password);", 10)).toBe("hashPassword");
  });
  it("returns null when there's no identifier under the cursor", () => {
    expect(pickIdentifier("return hashPassword();", 6)).toBe("return");
    expect(pickIdentifier("   ", 1)).toBeNull();
  });
});
