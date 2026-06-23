//! `git_log` via gix.

use crate::error::{ErrorCode, ToolError};
use crate::util::validate_ref;
use gix::bstr::BStr;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub email: String,
    pub date: i64,
    pub summary: String,
}

pub fn git_log(
    repo_path: &Path,
    ref_name: Option<&str>,
    file: Option<&str>,
    max_count: usize,
) -> Result<Vec<CommitInfo>, ToolError> {
    let repo = gix::open(repo_path)
        .map_err(|e| ToolError::new(ErrorCode::UnknownRepo, format!("open: {e}")))?;
    let start_id = if let Some(r) = ref_name {
        validate_ref(r)?;
        let bs: &BStr = r.into();
        repo.rev_parse_single(bs)
            .map_err(|e| ToolError::new(ErrorCode::UnknownRef, format!("rev-parse {r}: {e}")))?
            .detach()
    } else {
        repo.head_id()
            .map_err(|e| ToolError::new(ErrorCode::UnknownRef, format!("head: {e}")))?
            .detach()
    };

    let walk = repo
        .rev_walk(std::iter::once(start_id))
        .all()
        .map_err(|e| ToolError::internal(format!("rev_walk: {e}")))?;

    let mut out = Vec::new();
    for info in walk {
        if out.len() >= max_count {
            break;
        }
        let info = info.map_err(|e| ToolError::internal(format!("walk item: {e}")))?;
        let commit = repo
            .find_object(info.id)
            .map_err(|e| ToolError::internal(format!("find object: {e}")))?
            .try_into_commit()
            .map_err(|e| ToolError::internal(format!("not commit: {e}")))?;
        let message = commit
            .message()
            .map_err(|e| ToolError::internal(format!("msg: {e}")))?;
        let author_sig = commit
            .author()
            .map_err(|e| ToolError::internal(format!("author: {e}")))?;

        // File filter: only include if this commit touches the file. Cheap check via parent diff.
        if let Some(filter) = file {
            if !commit_touches_file(&repo, &commit, filter)? {
                continue;
            }
        }

        out.push(CommitInfo {
            sha: commit.id().to_hex().to_string(),
            author: author_sig.name.to_string(),
            email: author_sig.email.to_string(),
            date: author_sig.time.seconds,
            summary: message.summary().to_string(),
        });
    }

    Ok(out)
}

fn commit_touches_file(
    repo: &gix::Repository,
    commit: &gix::Commit,
    file: &str,
) -> Result<bool, ToolError> {
    let tree = commit
        .tree()
        .map_err(|e| ToolError::internal(format!("tree: {e}")))?;
    let mut buf: Vec<u8> = Vec::new();
    let here = tree
        .lookup_entry_by_path(file, &mut buf)
        .map_err(|e| ToolError::internal(format!("lookup: {e}")))?
        .map(|e| e.object_id());

    let parents: Vec<_> = commit.parent_ids().collect();
    if parents.is_empty() {
        return Ok(here.is_some());
    }
    for pid in parents {
        let parent = repo
            .find_object(pid)
            .map_err(|e| ToolError::internal(format!("parent: {e}")))?
            .try_into_commit()
            .map_err(|e| ToolError::internal(format!("parent not commit: {e}")))?;
        let ptree = parent
            .tree()
            .map_err(|e| ToolError::internal(format!("ptree: {e}")))?;
        let mut pbuf: Vec<u8> = Vec::new();
        let there = ptree
            .lookup_entry_by_path(file, &mut pbuf)
            .map_err(|e| ToolError::internal(format!("plookup: {e}")))?
            .map(|e| e.object_id());
        if here != there {
            return Ok(true);
        }
    }
    Ok(false)
}
