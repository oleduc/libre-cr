import { beforeEach, describe, expect, it } from "vitest";

import { scrapePr } from "../utils/github/scrape";

const FIXTURE = `
<div class="gh-header">
  <h1 class="gh-header-title"><span class="js-issue-title">feat: bcrypt migration</span></h1>
  <div class="gh-header-meta">
    <a class="author">octocat</a>
    <span class="commit-ref head-ref"><span class="css-truncate-target">feature/bcrypt</span></span>
    <span class="commit-ref base-ref"><span class="css-truncate-target">main</span></span>
  </div>
</div>
<div class="pull-discussion-timeline">
  <div class="markdown-body">Migrate from md5 to bcrypt for password hashing.</div>
</div>
<div class="file" data-tagsearch-path="src/auth.ts"></div>
<div class="file" data-tagsearch-path="src/users.ts"></div>
`;

describe("scrapePr", () => {
  beforeEach(() => {
    document.body.innerHTML = FIXTURE;
    // Patch location.pathname for owner/repo/number resolution.
    history.replaceState(null, "", "/octocat/repo/pull/42");
  });

  it("extracts title, description, author, branches, and files", () => {
    const out = scrapePr();
    expect(out.data.title).toBe("feat: bcrypt migration");
    expect(out.data.description).toContain("md5 to bcrypt");
    expect(out.data.author).toBe("octocat");
    expect(out.data.head_branch).toBe("feature/bcrypt");
    expect(out.data.base_branch).toBe("main");
    expect(out.data.files_changed).toEqual(["src/auth.ts", "src/users.ts"]);
    expect(out.data.owner).toBe("octocat");
    expect(out.data.repo).toBe("repo");
    expect(out.data.number).toBe(42);
  });

  it("yields nulls for missing fields without throwing", () => {
    document.body.innerHTML = "<div></div>";
    const out = scrapePr();
    expect(out.data.title).toBeNull();
    expect(out.data.description).toBeNull();
    expect(out.warnings.length).toBeGreaterThan(0);
  });

  it("extracts head_sha from the octolytics meta tag", () => {
    // Add a meta tag to <head>.
    const meta = document.createElement("meta");
    meta.setAttribute("name", "octolytics-dimension-pull_request_head_sha");
    meta.setAttribute("content", "deadbeefcafe0000");
    document.head.appendChild(meta);
    const out = scrapePr();
    expect(out.data.head_sha).toBe("deadbeefcafe0000");
    meta.remove();
  });
});
