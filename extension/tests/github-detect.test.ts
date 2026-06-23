import { describe, expect, it } from "vitest";

import { isPullRequestPage } from "../utils/github/detect";

describe("isPullRequestPage", () => {
  it("matches /owner/repo/pull/N", () => {
    expect(isPullRequestPage("/foo/bar/pull/123")).toEqual({
      owner: "foo",
      repo: "bar",
      number: 123,
    });
  });
  it("matches the /files sub-page", () => {
    expect(isPullRequestPage("/foo/bar/pull/4/files")).toEqual({
      owner: "foo",
      repo: "bar",
      number: 4,
    });
  });
  it("rejects non-PR paths", () => {
    expect(isPullRequestPage("/foo/bar")).toBeNull();
    expect(isPullRequestPage("/foo/bar/issues/1")).toBeNull();
    expect(isPullRequestPage("/")).toBeNull();
  });
  it("rejects a PR with a non-numeric id", () => {
    expect(isPullRequestPage("/foo/bar/pull/notanumber")).toBeNull();
  });
});
