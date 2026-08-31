import { describe, expect, it } from "vitest";

import { digestOfPath, selectionFromDiffHash } from "../utils/github/gh-selection";

describe("GitHub diff-hash selection", () => {
  it("decodes single lines and shift-click ranges via the path digest", async () => {
    document.body.innerHTML =
      '<table aria-label="Diff for: crates/globset/Cargo.toml"><tr><td data-line-number="34">34</td></tr></table>';
    const digest = await digestOfPath("crates/globset/Cargo.toml");
    expect(digest).toBe("96dcf1342451b645a85b47028d85be9d50edfa7e1c8b105cc9b7ac6d732b9b0c");

    expect(await selectionFromDiffHash(`#diff-${digest}R34`)).toEqual({
      kind: "line",
      file: "crates/globset/Cargo.toml",
      line: 34,
    });
    expect(await selectionFromDiffHash(`#diff-${digest}R34-R36`)).toEqual({
      kind: "range",
      file: "crates/globset/Cargo.toml",
      start_line: 34,
      end_line: 36,
    });
    // Old-side lines and unknown digests
    expect(await selectionFromDiffHash(`#diff-${digest}L12`)).toEqual({
      kind: "line",
      file: "crates/globset/Cargo.toml",
      line: 12,
    });
    expect(await selectionFromDiffHash(`#diff-${"0".repeat(64)}R1`)).toBeNull();
    expect(await selectionFromDiffHash("#readme")).toBeNull();
  });
});
