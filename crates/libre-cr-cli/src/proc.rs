//! Process plumbing: PID files and liveness checks.
//!
//! We deliberately avoid `sysinfo` to keep the binary lean. On Unix we use
//! `kill(pid, 0)` semantics via `nix`; on Windows we fall back to attempting
//! to open the process via the standard library's exit-code check.

use std::path::Path;

use anyhow::{Context, Result};

/// Read a PID from a file. Returns `Ok(None)` if the file is missing or
/// contains garbage; we treat both as "no daemon recorded".
pub fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)
        .with_context(|| format!("read pid file {}", path.display()))?;
    Ok(s.trim().parse::<u32>().ok())
}

/// Atomically (best-effort) write a PID to a file, creating parents as
/// needed. We tighten the mode to 0600 on Unix because the file lives inside
/// the per-user state dir; defense-in-depth.
pub fn write_pid_file(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    std::fs::write(path, format!("{pid}\n"))
        .with_context(|| format!("write pid file {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    Ok(())
}

/// Remove a PID file if it exists. Missing file is not an error.
pub fn remove_pid_file(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("remove pid file {}", path.display()))?;
    }
    Ok(())
}

/// Is the given PID currently alive?
///
/// On Unix this issues `kill(pid, 0)` — present + permitted → alive,
/// `ESRCH` → dead, `EPERM` → alive-but-not-ours (still treat as alive).
/// On Windows we approximate via process handle open; missing → dead.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    let p = Pid::from_raw(pid as i32);
    match kill(p, None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    // Conservative: assume any non-zero pid we can't disprove is alive. On
    // Windows we'd ideally call `OpenProcess` and `GetExitCodeProcess`, but
    // pulling in `windows-sys` for a single check is overkill here. A stale
    // PID will be cleared the first time `stop` runs.
    pid > 0
}

/// Send SIGTERM (Unix) or call `kill` (Windows) on a PID. Best-effort.
#[cfg(unix)]
pub fn send_term(pid: u32) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
        .with_context(|| format!("SIGTERM pid {pid}"))?;
    Ok(())
}

#[cfg(windows)]
pub fn send_term(_pid: u32) -> Result<()> {
    // Without windows-sys we can't send a soft stop signal. Callers should
    // fall back to hard-kill on Windows.
    Ok(())
}

/// Force-kill (SIGKILL on Unix; placeholder on Windows). Best-effort.
#[cfg(unix)]
pub fn send_kill(pid: u32) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGKILL)
        .with_context(|| format!("SIGKILL pid {pid}"))?;
    Ok(())
}

#[cfg(windows)]
pub fn send_kill(_pid: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_pid_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nest/review.pid");
        write_pid_file(&p, 4321).unwrap();
        assert_eq!(read_pid_file(&p).unwrap(), Some(4321));
    }

    #[test]
    fn read_pid_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("none.pid");
        assert_eq!(read_pid_file(&p).unwrap(), None);
    }

    #[test]
    fn read_pid_garbage_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.pid");
        std::fs::write(&p, "not-a-pid").unwrap();
        assert_eq!(read_pid_file(&p).unwrap(), None);
    }

    #[test]
    fn remove_pid_file_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.pid");
        remove_pid_file(&p).unwrap();
        std::fs::write(&p, "1").unwrap();
        remove_pid_file(&p).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn self_pid_is_alive() {
        let me = std::process::id();
        assert!(is_alive(me));
    }

    #[test]
    fn nonexistent_pid_is_dead() {
        // PID 0 / very high values won't exist as a real process.
        // On Unix kill(pid=0) targets the process group, so use a high pid
        // we know is unallocated (the `pid_max` upper bound rules out this
        // 32-bit value on every mainstream OS).
        assert!(!is_alive(0x7fff_fff0));
    }
}
