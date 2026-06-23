//! Read files / list tree entries at a ref via gix.

use crate::error::{ErrorCode, ToolError};
use gix::bstr::BStr;
use std::path::{Path, PathBuf};

fn open_at(repo_path: &Path) -> Result<gix::Repository, ToolError> {
    gix::open(repo_path)
        .map_err(|e| ToolError::new(ErrorCode::UnknownRepo, format!("open repo: {e}")))
}

fn parse_ref<'r>(repo: &'r gix::Repository, ref_name: &str) -> Result<gix::Id<'r>, ToolError> {
    let spec: &BStr = ref_name.into();
    repo.rev_parse_single(spec)
        .map_err(|e| ToolError::new(ErrorCode::UnknownRef, format!("rev-parse {ref_name}: {e}")))
}

/// Read the full contents of a path at the given ref.
pub fn read_blob_at_ref(
    repo_path: &Path,
    ref_name: &str,
    file: &str,
) -> Result<Vec<u8>, ToolError> {
    let repo = open_at(repo_path)?;
    let id = parse_ref(&repo, ref_name)?;
    let obj = id
        .object()
        .map_err(|e| ToolError::internal(format!("object lookup: {e}")))?;
    let commit = obj
        .try_into_commit()
        .map_err(|e| ToolError::new(ErrorCode::UnknownRef, format!("not a commit: {e}")))?;
    let tree = commit
        .tree()
        .map_err(|e| ToolError::internal(format!("tree: {e}")))?;
    let mut buf: Vec<u8> = Vec::new();
    let entry = tree
        .lookup_entry_by_path(file, &mut buf)
        .map_err(|e| ToolError::internal(format!("lookup: {e}")))?
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::NotInWorkspace,
                format!("file {file} not found at ref {ref_name}"),
            )
        })?;
    let blob = entry
        .object()
        .map_err(|e| ToolError::internal(format!("blob: {e}")))?;
    Ok(blob.data.clone())
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub kind: &'static str, // "file" or "dir"
    pub size: Option<u64>,
}

/// List a directory at the given ref.
pub fn list_dir_at_ref(
    repo_path: &Path,
    ref_name: &str,
    dir: &str,
) -> Result<Vec<TreeEntry>, ToolError> {
    let repo = open_at(repo_path)?;
    let id = parse_ref(&repo, ref_name)?;
    let obj = id
        .object()
        .map_err(|e| ToolError::internal(format!("object lookup: {e}")))?;
    let commit = obj
        .try_into_commit()
        .map_err(|e| ToolError::new(ErrorCode::UnknownRef, format!("not a commit: {e}")))?;
    let mut tree = commit
        .tree()
        .map_err(|e| ToolError::internal(format!("tree: {e}")))?;

    let normalized = dir.trim_start_matches('/').trim_end_matches('/');
    if !normalized.is_empty() && normalized != "." {
        let mut buf: Vec<u8> = Vec::new();
        let sub = tree
            .lookup_entry_by_path(normalized, &mut buf)
            .map_err(|e| ToolError::internal(format!("lookup: {e}")))?
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::NotInWorkspace,
                    format!("dir not found: {normalized}"),
                )
            })?;
        let obj = sub
            .object()
            .map_err(|e| ToolError::internal(format!("obj: {e}")))?;
        tree = obj
            .try_into_tree()
            .map_err(|e| ToolError::internal(format!("not a tree: {e}")))?;
    }

    let mut out = Vec::new();
    for entry in tree.iter() {
        let entry = entry.map_err(|e| ToolError::internal(format!("iter: {e}")))?;
        let mode = entry.mode();
        let kind = if mode.is_tree() { "dir" } else { "file" };
        out.push(TreeEntry {
            name: entry.filename().to_string(),
            kind,
            size: None,
        });
    }
    Ok(out)
}

/// Walk a working-tree dir (no ref).
pub fn list_dir_working_tree(dir: &Path) -> Result<Vec<TreeEntry>, ToolError> {
    let mut out = Vec::new();
    let read = std::fs::read_dir(dir)
        .map_err(|e| ToolError::new(ErrorCode::NotInWorkspace, format!("read_dir: {e}")))?;
    for entry in read {
        let entry = entry?;
        let meta = entry.metadata()?;
        let kind = if meta.is_dir() { "dir" } else { "file" };
        let size = if meta.is_file() {
            Some(meta.len())
        } else {
            None
        };
        out.push(TreeEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind,
            size,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Open as a gix repo and return the working directory.
pub fn discover_repo_root(start: &Path) -> Result<PathBuf, ToolError> {
    let repo = gix::discover(start)
        .map_err(|e| ToolError::new(ErrorCode::UnknownRepo, format!("not a git repo: {e}")))?;
    Ok(repo
        .work_dir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.to_path_buf()))
}

/// Return the remote URLs from `.git/config`.
pub fn remotes(repo_path: &Path) -> Result<Vec<String>, ToolError> {
    let repo = open_at(repo_path)?;
    let names = repo
        .remote_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for name in names {
        let name_bs: &BStr = name.as_str().into();
        if let Ok(remote) = repo.find_remote(name_bs) {
            if let Some(url) = remote.url(gix::remote::Direction::Fetch) {
                out.push(url.to_bstring().to_string());
            }
        }
    }
    Ok(out)
}

/// Resolve HEAD's symbolic ref short name (e.g. "main"). Best-effort.
pub fn default_branch(repo_path: &Path) -> Option<String> {
    let repo = gix::open(repo_path).ok()?;
    let head = repo.head().ok()?;
    head.referent_name().map(|n| n.shorten().to_string())
}
