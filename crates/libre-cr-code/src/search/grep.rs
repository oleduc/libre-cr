//! Text search via the ripgrep `grep-*` library crates.

use crate::error::ToolError;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Match {
    pub file: String,
    pub line: u64,
    pub column: u64,
    pub content: String,
}

pub struct GrepOptions<'a> {
    pub pattern: &'a str,
    pub paths: Option<&'a [String]>,
    pub glob: Option<&'a str>,
    pub fixed_string: bool,
    pub max_matches: usize,
}

pub fn search(repo_path: &Path, opts: &GrepOptions) -> Result<(Vec<Match>, bool), ToolError> {
    let pat = if opts.fixed_string {
        regex::escape(opts.pattern)
    } else {
        opts.pattern.to_string()
    };
    let matcher = RegexMatcher::new(&pat).map_err(|e| ToolError::invalid(format!("regex: {e}")))?;

    let mut matches: Vec<Match> = Vec::new();
    let mut truncated = false;

    let mut builder = ignore::WalkBuilder::new(repo_path);
    builder.git_ignore(true).hidden(true);
    if let Some(globs) = opts.glob {
        let mut ob = ignore::overrides::OverrideBuilder::new(repo_path);
        ob.add(globs)
            .map_err(|e| ToolError::invalid(format!("glob: {e}")))?;
        let over = ob
            .build()
            .map_err(|e| ToolError::invalid(format!("glob build: {e}")))?;
        builder.overrides(over);
    }

    let restrict: Option<Vec<PathBuf>> = opts
        .paths
        .map(|ps| ps.iter().map(|p| repo_path.join(p)).collect());

    let walker = builder.build();
    for entry in walker {
        if matches.len() >= opts.max_matches {
            truncated = true;
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(rs) = &restrict {
            if !rs.iter().any(|r| path.starts_with(r)) {
                continue;
            }
        }

        let rel = path
            .strip_prefix(repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();

        let remaining = opts.max_matches - matches.len();
        let mut sink = LineSink {
            file: rel,
            out: &mut matches,
            remaining,
            hit_cap: false,
        };
        let mut searcher = Searcher::new();
        let _ = searcher.search_path(&matcher, path, &mut sink);
        if sink.hit_cap {
            truncated = true;
            break;
        }
    }

    Ok((matches, truncated))
}

struct LineSink<'a> {
    file: String,
    out: &'a mut Vec<Match>,
    remaining: usize,
    hit_cap: bool,
}

impl Sink for LineSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.remaining == 0 {
            self.hit_cap = true;
            return Ok(false);
        }
        let line = mat.line_number().unwrap_or(0);
        let content = String::from_utf8_lossy(mat.bytes())
            .trim_end_matches('\n')
            .to_string();
        self.out.push(Match {
            file: self.file.clone(),
            line,
            column: 1,
            content,
        });
        self.remaining -= 1;
        if self.remaining == 0 {
            self.hit_cap = true;
            return Ok(false);
        }
        Ok(true)
    }
}

// Tiny inline regex-escape helper since we don't depend on `regex` crate.
mod regex {
    pub fn escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^'
                | '$' | '#' | '-' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello\nworld\nhello again\n").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "foo bar\nbaz\n").unwrap();
        tmp
    }

    #[test]
    fn finds_pattern() {
        let tmp = fixture();
        let opts = GrepOptions {
            pattern: "hello",
            paths: None,
            glob: None,
            fixed_string: true,
            max_matches: 200,
        };
        let (matches, truncated) = search(tmp.path(), &opts).unwrap();
        assert!(matches.len() >= 2);
        assert!(!truncated);
    }

    #[test]
    fn truncates_at_cap() {
        let tmp = TempDir::new().unwrap();
        let mut text = String::new();
        for _ in 0..50 {
            text.push_str("xx\n");
        }
        std::fs::write(tmp.path().join("f.txt"), text).unwrap();
        let opts = GrepOptions {
            pattern: "xx",
            paths: None,
            glob: None,
            fixed_string: true,
            max_matches: 5,
        };
        let (matches, truncated) = search(tmp.path(), &opts).unwrap();
        assert_eq!(matches.len(), 5);
        assert!(truncated);
    }

    #[test]
    fn regex_works() {
        let tmp = fixture();
        let opts = GrepOptions {
            pattern: r"^foo",
            paths: None,
            glob: None,
            fixed_string: false,
            max_matches: 200,
        };
        let (matches, _) = search(tmp.path(), &opts).unwrap();
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.content.starts_with("foo")));
    }
}
