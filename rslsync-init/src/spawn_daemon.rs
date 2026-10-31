//! Rootless-aware daemon spawning, privilege dropping (`setpriv`), and 10s SQLite WAL shutdown loop.

use std::fs;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, SystemTime};

pub struct DaemonContext<'a> {
    pub puid: u32,
    pub pgid: u32,
    pub umask_val: u32,
    pub is_root: bool,
    pub drop_privs: bool,
    pub seed_ignores: bool,
    pub sync_conf_path: &'a str,
}

pub fn parse_octal_umask(s: &str) -> u32 {
    u32::from_str_radix(s.trim(), 8).unwrap_or(0o002)
}

pub fn format_umask(umask_val: u32) -> String {
    format!("{umask_val:03o}")
}

pub fn read_effective_uid() -> u32 {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let mut parts = rest.split_whitespace();
            let _real = parts.next();
            if let Some(eff) = parts.next() {
                if let Ok(uid) = eff.parse::<u32>() {
                    return uid;
                }
            }
        }
    }
    0
}

pub fn find_rslsync_bin() -> &'static str {
    ["/bin/rslsync", "/usr/bin/rslsync"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .unwrap_or("/bin/rslsync")
}

pub fn run_validator(sync_conf_path: &str) -> bool {
    let mut cmd = Command::new("/bin/rslsync-config");
    cmd.args(["--config", sync_conf_path]);
    matches!(cmd.status(), Ok(s) if s.success())
}

pub fn spawn_daemon_process(
    prog: &str,
    args: &[&str],
    puid: u32,
    pgid: u32,
    umask_val: u32,
    is_root: bool,
    drop_privs: bool,
) -> Result<Child, String> {
    if is_root && drop_privs && puid != 0 {
        let uid_arg = format!("--reuid={puid}");
        let gid_arg = format!("--regid={pgid}");

        let mut setpriv_cmd = Command::new("setpriv");
        setpriv_cmd
            .arg(&uid_arg)
            .arg(&gid_arg)
            .arg("--clear-groups")
            .arg("--")
            .arg(prog)
            .args(args)
            .env("HOME", "/config")
            .env("UMASK", format_umask(umask_val));

        if let Ok(mut child) = setpriv_cmd.spawn() {
            thread::sleep(Duration::from_millis(50));
            if matches!(child.try_wait(), Ok(None)) {
                return Ok(child);
            }
        }

        let mut fallback = Command::new(prog);
        fallback
            .args(args)
            .env("HOME", "/config")
            .env("UMASK", format_umask(umask_val))
            .uid(puid)
            .gid(pgid);
        return fallback
            .spawn()
            .map_err(|e| format!("Failed to spawn unprivileged {prog}: {e}"));
    }

    let mut cmd = Command::new(prog);
    cmd.args(args)
        .env("HOME", "/config")
        .env("UMASK", format_umask(umask_val));
    cmd.spawn()
        .map_err(|e| format!("Failed to spawn {prog}: {e}"))
}

pub fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

pub fn terminate_children(children: &mut [Child]) {
    for child in children.iter() {
        let pid_str = child.id().to_string();
        let _ = Command::new("kill").args(["-TERM", &pid_str]).status();
    }
    // Poll up to 10 seconds (20 x 500ms) so rslsync checkpoints SQLite storage.db-wal cleanly
    for _ in 0..20 {
        let all_exited = children
            .iter_mut()
            .all(|c| matches!(c.try_wait(), Ok(Some(_))));
        if all_exited {
            return;
        }
        thread::sleep(Duration::from_millis(500));
    }
    for child in children.iter_mut() {
        let _ = child.kill();
        if let Ok(st) = child.wait() {
            let _ = st.signal();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_umask_and_effective_uid() {
        assert_eq!(parse_octal_umask("002"), 0o002);
        assert_eq!(parse_octal_umask("022"), 0o022);
        assert_eq!(parse_octal_umask("invalid"), 0o002);
        assert_eq!(format_umask(0o002), "002");
        let _ = read_effective_uid();
    }
}
