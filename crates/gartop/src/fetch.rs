//! Running `gartop-fetch` in the background from the viewer.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use chrono::{DateTime, Local};

/// Exit code `gartop-fetch` uses when another fetch holds the lock.
const EXIT_LOCKED: i32 = 3;

/// How the most recent manual fetch went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Ok,
    AlreadyRunning,
    Failed(String),
}

/// State of the manual-fetch feature.
#[derive(Debug)]
pub enum FetchState {
    Idle,
    Running {
        started: Instant,
        rx: Receiver<FetchOutcome>,
    },
    Done {
        at: DateTime<Local>,
        outcome: FetchOutcome,
    },
}

/// Locate the fetcher: an explicit override, else next to this executable, else `PATH`.
pub fn fetcher_program(override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return p;
    }
    let name = format!("gartop-fetch{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(&name))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Spawn `<program> sync --days 2` on a worker thread.
pub fn spawn(program: PathBuf) -> FetchState {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = match Command::new(&program)
            .args(["sync", "--days", "2"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(out) if out.status.success() => FetchOutcome::Ok,
            Ok(out) if out.status.code() == Some(EXIT_LOCKED) => FetchOutcome::AlreadyRunning,
            Ok(out) => FetchOutcome::Failed(last_line(&String::from_utf8_lossy(&out.stderr))),
            Err(e) => FetchOutcome::Failed(format!("could not run {}: {e}", program.display())),
        };
        let _ = tx.send(outcome);
    });
    FetchState::Running {
        started: Instant::now(),
        rx,
    }
}

/// Advance a running fetch. Returns `true` when it just finished.
pub fn poll(state: &mut FetchState) -> bool {
    let FetchState::Running { rx, .. } = state else {
        return false;
    };
    let outcome = match rx.try_recv() {
        Ok(o) => o,
        Err(TryRecvError::Empty) => return false,
        Err(TryRecvError::Disconnected) => FetchOutcome::Failed("fetch worker died".into()),
    };
    *state = FetchState::Done {
        at: Local::now(),
        outcome,
    };
    true
}

fn last_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("gartop-fetch failed");
    strip_ansi(line)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait(mut state: FetchState) -> FetchOutcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !poll(&mut state) {
            assert!(Instant::now() < deadline, "fetch did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        match state {
            FetchState::Done { outcome, .. } => outcome,
            _ => unreachable!(),
        }
    }

    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn maps_exit_codes() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(
            dir.path(),
            "ok",
            r#"[ "$1 $2 $3" = "sync --days 2" ] || exit 9"#,
        );
        assert_eq!(wait(spawn(ok)), FetchOutcome::Ok);

        let locked = script(dir.path(), "locked", "exit 3");
        assert_eq!(wait(spawn(locked)), FetchOutcome::AlreadyRunning);

        let failed = script(
            dir.path(),
            "failed",
            r"printf '\033[31mERROR\033[0m boom\n\n' >&2; exit 1",
        );
        assert_eq!(
            wait(spawn(failed)),
            FetchOutcome::Failed("ERROR boom".into())
        );
    }

    #[test]
    fn missing_program_is_reported() {
        let outcome = wait(spawn(PathBuf::from("/definitely/not/here")));
        assert!(matches!(outcome, FetchOutcome::Failed(m) if m.contains("could not run")));
    }
}
