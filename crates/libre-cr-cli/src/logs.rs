//! Append-only log writers and a `tail`-equivalent reader.
//!
//! Daily rotation (spec § Supervision Model) is a TODO; the wrapper opens
//! the file in append mode for every write, which keeps the implementation
//! trivial and safe across crashes. A future change can wire in rotation
//! without touching call sites.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};

use crate::paths;

/// Append a timestamped line to `supervisor.log`.
pub async fn supervisor_event(line: impl AsRef<str>) -> Result<()> {
    append(&paths::supervisor_log_file(), line.as_ref()).await
}

/// Append a single line to a log file. Creates parents as needed.
pub async fn append(path: &Path, line: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("open log {}", path.display()))?;
    let stamp = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    let trimmed = line.trim_end_matches('\n');
    f.write_all(format!("{stamp} {trimmed}\n").as_bytes())
        .await?;
    f.flush().await?;
    Ok(())
}

/// Tail one or more files. With `follow = false`, prints the last `lines`
/// lines per file, merged in a stable order (by ISO-8601 prefix when one is
/// present, otherwise file order). With `follow = true`, streams new lines
/// indefinitely; the caller cancels by dropping the future.
pub async fn tail(files: &[PathBuf], lines: usize, follow: bool) -> Result<()> {
    if !follow {
        let mut merged: Vec<(String, String)> = Vec::new();
        for f in files {
            for line in read_last_lines(f, lines).await? {
                let label = f
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                merged.push((line, label));
            }
        }
        // Sort by the (likely-timestamp) line prefix.
        merged.sort_by(|a, b| a.0.cmp(&b.0));
        for (line, label) in merged {
            println!("[{label}] {line}");
        }
        return Ok(());
    }

    // Follow mode: open each file at EOF and poll for new bytes.
    let mut readers = Vec::new();
    for f in files {
        // Touch the file so we have something to follow even on first run.
        if !f.exists() {
            if let Some(parent) = f.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::File::create(f).await?;
        }
        let mut file = tokio::fs::File::open(f).await?;
        file.seek(std::io::SeekFrom::End(0)).await?;
        let label = f
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        readers.push((label, BufReader::new(file)));
    }

    loop {
        let mut any = false;
        for (label, reader) in &mut readers {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await.unwrap_or(0);
            if n > 0 {
                any = true;
                let trimmed = line.trim_end_matches('\n');
                println!("[{label}] {trimmed}");
            }
        }
        if !any {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Read the last `n` lines of a file. Cheap, naive implementation: read the
/// whole file. Log files are size-bounded by retention so this is fine.
async fn read_last_lines(path: &Path, n: usize) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = tokio::fs::read_to_string(path).await?;
    let lines: VecDeque<&str> = data.lines().collect();
    let skip = lines.len().saturating_sub(n);
    Ok(lines.iter().skip(skip).map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn append_then_read_lines() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.log");
        for i in 0..5 {
            append(&f, &format!("line {i}")).await.unwrap();
        }
        let last = read_last_lines(&f, 3).await.unwrap();
        assert_eq!(last.len(), 3);
        assert!(last[0].contains("line 2"));
        assert!(last[2].contains("line 4"));
    }

    #[tokio::test]
    async fn append_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("nested/dir/log.txt");
        append(&f, "hi").await.unwrap();
        assert!(f.exists());
    }

    #[tokio::test]
    async fn read_last_lines_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("nope.log");
        let last = read_last_lines(&f, 10).await.unwrap();
        assert!(last.is_empty());
    }
}
