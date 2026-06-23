// GitHub DOM selectors, isolated in one place so a refresh can edit a
// single file. None of these are stable contracts; PRs from upstream may
// require a tweak.

export const SELECTORS = {
  prHeaderTitle: ".gh-header-title .js-issue-title, h1.gh-header-title bdi",
  prHeaderMeta: ".gh-header-meta",
  prDescription: ".pull-discussion-timeline .markdown-body, .pull-discussion-timeline .comment-body",
  prAuthor: ".gh-header-meta .author, .gh-header-meta a.author",
  baseBranch: ".commit-ref.base-ref .css-truncate-target, .base-ref .css-truncate-target",
  headBranch: ".commit-ref.head-ref .css-truncate-target, .head-ref .css-truncate-target",
  filesChangedTab: "a#files_tab .Counter, a.js-pull-request-tab[href*='/files'] .Counter",
  fileRowsInDiff: ".file[data-tagsearch-path]",
  diffTableRow: "tr.js-file-line-container > tr, table.diff-table tr",
  diffLineNumber: "td.blob-num.js-line-number, td.blob-num",
  diffLineCode: "td.blob-code",
  // PR head ref SHA. GitHub injects this as a meta tag on PR pages; we read
  // it so the daemon can detect when the PR's commits have changed across
  // reopens.
  headShaMeta:
    "meta[name='octolytics-dimension-pull_request_head_sha'], meta[name='octolytics-pull_request_head_sha']",
};

export const SELECTOR_VERSION = 1;
