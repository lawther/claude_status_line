// Cache-expiry alarm: fires a macOS notification a few minutes before the
// prompt cache expires. Claude Code re-invokes claude_status_line on every
// turn, so each invocation just rewrites a small "deadline" file for the
// session; a single long-lived watcher process (spawned once per session)
// polls that file and fires the notification when the deadline passes. The
// watcher exits on its own once the Claude Code process that started the
// session is gone, so there is nothing to clean up on quit.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::now_secs;

const WARNING_LEAD_SECS: u64 = 10 * 60;
const POLL_INTERVAL: Duration = Duration::from_secs(15);

fn state_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join(".claude/statusline-alarms");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn deadline_path(session_id: &str, dir: &std::path::Path) -> PathBuf {
    dir.join(format!("{session_id}.deadline"))
}

fn watcher_pid_path(session_id: &str, dir: &std::path::Path) -> PathBuf {
    dir.join(format!("{session_id}.watcher-pid"))
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub(crate) fn parent_pid() -> Option<u32> {
    let own_pid = std::process::id();
    let output = Command::new("ps")
        .args(["-o", "ppid=", "-p", &own_pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

// `osascript -e 'display notification'` is an unreliable legacy AppleScript
// shim on modern macOS when invoked from a non-Apple terminal: it has no
// registered app identity for Notification Center to attribute the alert
// to, so it is often silently dropped rather than prompting for permission.
// `terminal-notifier` is a small signed app that registers itself properly
// and works reliably; see `just setup-cache-alarm` for the one-time install.
// If it isn't installed, we skip the notification rather than fail.
fn notify(session_name: &str) {
    let message = format!("Prompt cache for \"{session_name}\" expires in under 10 minutes");
    let _ = Command::new("terminal-notifier")
        .args([
            "-title",
            "Claude Code",
            "-message",
            &message,
            "-sound",
            "Glass",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Called on every normal invocation. Records the current cache-expiry
/// deadline for this session and makes sure a watcher is running to act on
/// it. Best-effort throughout: any failure here must never affect the
/// status line's own output.
pub fn arm(session_id: &str, session_name: &str, cache_expires_at: u64, claude_pid: u32) {
    let Some(dir) = state_dir() else { return };

    let deadline = cache_expires_at.saturating_sub(WARNING_LEAD_SECS);
    if let Ok(mut f) = fs::File::create(deadline_path(session_id, &dir)) {
        let _ = writeln!(f, "{deadline}");
    }

    let watcher_pid_file = watcher_pid_path(session_id, &dir);
    let already_running = fs::read_to_string(&watcher_pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(process_alive);

    if already_running {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    if let Ok(child) = Command::new(exe)
        .arg("--watch-cache-alarm")
        .arg(session_id)
        .arg(session_name)
        .arg(claude_pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let _ = fs::write(&watcher_pid_file, child.id().to_string());
    }
}

/// Entry point for the detached watcher process (`--watch-cache-alarm
/// <session_id> <session_name> <claude_pid>`). Polls the deadline file,
/// fires a notification once when the deadline passes, and exits as soon as
/// the owning Claude Code process is no longer running.
pub fn run_watcher(session_id: &str, session_name: &str, claude_pid: u32) {
    let Some(dir) = state_dir() else { return };
    let deadline_file = deadline_path(session_id, &dir);
    let watcher_pid_file = watcher_pid_path(session_id, &dir);

    let mut last_seen_deadline: Option<u64> = None;
    let mut fired_for: Option<u64> = None;

    loop {
        if !process_alive(claude_pid) {
            let _ = fs::remove_file(&deadline_file);
            let _ = fs::remove_file(&watcher_pid_file);
            return;
        }

        if let Some(deadline) = fs::read_to_string(&deadline_file)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            if last_seen_deadline != Some(deadline) {
                last_seen_deadline = Some(deadline);
                // A deadline that moved forward again (cache extended by a new
                // turn) means any previous firing no longer applies.
                if fired_for != Some(deadline) {
                    fired_for = None;
                }
            }

            let now = now_secs().unwrap_or(0);
            if now >= deadline && fired_for != Some(deadline) {
                notify(session_name);
                fired_for = Some(deadline);
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}
