import { describe, expect, it, beforeEach } from "vitest";

import { enumerateDiffLines, ensureFileRendered, findRow, hitTestLine } from "../utils/github/diff";
import { scrapePr } from "../utils/github/scrape";

// Trimmed from a live github.com/<owner>/<repo>/pull/<n>/changes page (the
// React UI GitHub redirects /files to). Classic-DOM coverage lives in
// github-diff.test.ts / github-scrape.test.ts.
const REACT_UI_FIXTURE = `
<script type="application/json" data-target="react-app.embeddedData">{"payload":{"comparison":{"fullDiff":{"baseOid":"7525479a9576f1ca4c2d04339d78e47ff5ae9b05","headOid":"bd287d51a90d662518bcc96c9df3835aea0dc75c"}}}}</script>
<h1 class="prc-PageHeader-Title"><span class="f1 text-normal markdown-title">increase pool capacity</span><span class="sr-only">- #3502</span></h1>
<div class="d-flex"><a class="PullRequestBranchName-module__branchName__SCtl2" href="/o/r/tree/master">master</a><a class="PullRequestBranchName-module__branchName__SCtl2" href="/o/r/tree/ag/pool">ag/pool</a></div>
<div id="diff-96dc" role="region">
  <table aria-label="Diff for: crates/globset/Cargo.toml" role="grid">
    <tr class="diff-line-row">
      <td class="focusable-grid-cell new-diff-line-number" data-diff-side="left" data-line-number="35">35</td>
      <td class="focusable-grid-cell new-diff-line-number" data-diff-side="right" data-line-number="35">35</td>
      <td class="diff-text-cell focusable-grid-cell" data-diff-side="right" data-line-number="35">[dependencies.regex-automata]</td>
    </tr>
    <tr class="diff-line-row">
      <td class="focusable-grid-cell new-diff-line-number" data-diff-side="left" data-line-number="36">36</td>
      <td class="focusable-grid-cell new-diff-line-number empty-diff-line"></td>
      <td class="diff-text-cell focusable-grid-cell" data-diff-side="left" data-line-number="36">-version = "0.4.0"</td>
    </tr>
    <tr class="diff-line-row">
      <td class="focusable-grid-cell new-diff-line-number empty-diff-line"></td>
      <td class="focusable-grid-cell new-diff-line-number" data-diff-side="right" data-line-number="36">36</td>
      <td class="diff-text-cell focusable-grid-cell" data-diff-side="right" data-line-number="36">+version = "0.4.18"</td>
    </tr>
  </table>
</div>
`;

describe("GitHub React 'changes' UI", () => {
  beforeEach(() => {
    document.body.innerHTML = REACT_UI_FIXTURE;
    history.replaceState(null, "", "/o/r/pull/3502/changes");
  });

  it("scrapes title, base/head, head SHA, and files", () => {
    const { data, warnings } = scrapePr();
    expect(data.title).toBe("increase pool capacity");
    expect(data.base_branch).toBe("master");
    expect(data.head_branch).toBe("ag/pool");
    expect(data.head_sha).toBe("bd287d51a90d662518bcc96c9df3835aea0dc75c");
    expect(data.files_changed).toEqual(["crates/globset/Cargo.toml"]);
    expect(warnings.filter((w) => /selectors may need refresh/.test(w))).toEqual([]);
  });

  it("enumerates lines with sides and hit-tests / finds rows", () => {
    const lines = enumerateDiffLines();
    expect(lines.map((l) => [l.line, l.side])).toEqual([
      [35, "R"],
      [36, "L"],
      [36, "R"],
    ]);
    expect(lines[0]!.file).toBe("crates/globset/Cargo.toml");

    const delCode = document.querySelectorAll(".diff-text-cell")[1]!.firstChild;
    expect(hitTestLine(delCode)).toEqual({ file: "crates/globset/Cargo.toml", line: 36, side: "L" });

    // Prefers the right/new side when both carry the number.
    const row = findRow("crates/globset/Cargo.toml", 36)!;
    expect(row.textContent).toContain("0.4.18");
    expect(findRow("crates/globset/Cargo.toml", 35)?.textContent).toContain("regex-automata");
    expect(findRow("nope.rs", 35)).toBeNull();
  });

  it("forces a virtualized file to render before targeting it", async () => {
    document.body.innerHTML = `
      <h3 id="heading-x">\u200eCargo.lock\u200e</h3>
      <div id="diff-abc" role="region" aria-labelledby="heading-x" data-estimated-height="4067"></div>`;
    const region = document.getElementById("diff-abc")!;
    // jsdom has no scrollIntoView; GitHub mounts the table once the region is scrolled to.
    (region as unknown as { scrollIntoView: () => void }).scrollIntoView = () => {
      setTimeout(() => {
        region.innerHTML =
          '<table aria-label="Diff for: Cargo.lock"><tr class="diff-line-row"><td data-diff-side="right" data-line-number="3">3</td></tr></table>';
      }, 50);
    };
    expect(findRow("Cargo.lock", 3)).toBeNull();
    expect(await ensureFileRendered("Cargo.lock")).toBe(true);
    expect(findRow("Cargo.lock", 3)).not.toBeNull();
    expect(await ensureFileRendered("nope.txt")).toBe(false);
  });
});
