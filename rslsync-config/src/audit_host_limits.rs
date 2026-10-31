//! Host filesystem existence checks (`storage_path`, `pid_file` parent) and kernel `inotify` audit.

use std::fs;
use std::path::Path;

use crate::validate_sync_conf::ValidatedSyncConf;

pub const RECOMMENDED_MIN_INOTIFY_WATCHES: u64 = 65_536;

pub fn read_inotify_max_watches_from(path: &Path) -> Option<u64> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse::<u64>().ok()
}

pub fn audit_storage_and_paths(cfg: &ValidatedSyncConf) -> Result<(), String> {
    let storage = Path::new(&cfg.storage_path);
    if !storage.exists() {
        return Err(format!(
            "storage_path '{}' does not exist (rslsync exits immediately if missing)",
            cfg.storage_path
        ));
    }
    if !storage.is_dir() {
        return Err(format!(
            "storage_path '{}' is not a directory",
            cfg.storage_path
        ));
    }

    let pid_path = Path::new(&cfg.pid_file);
    if let Some(parent) = pid_path.parent() {
        if !parent.exists() || !parent.is_dir() {
            return Err(format!(
                "pid_file parent directory '{}' does not exist",
                parent.display()
            ));
        }
    }

    let sync_root = Path::new(&cfg.directory_root);
    if !sync_root.exists() || !sync_root.is_dir() {
        return Err(format!(
            "directory_root '{}' does not exist or is not a directory",
            cfg.directory_root
        ));
    }

    let probe_file = storage.join(".rslsync-config-write-probe");
    if let Err(e) = fs::write(&probe_file, b"ok") {
        return Err(format!(
            "storage_path '{}' is not writable: {e}",
            cfg.storage_path
        ));
    }
    let _ = fs::remove_file(&probe_file);

    Ok(())
}

pub fn check_inotify_watch_budget() -> Option<(u64, bool)> {
    let limit = read_inotify_max_watches_from(Path::new("/proc/sys/fs/inotify/max_user_watches"))?;
    let sufficient = limit >= RECOMMENDED_MIN_INOTIFY_WATCHES;
    Some((limit, sufficient))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_storage_and_inotify_parser() {
        let tmp = std::env::temp_dir().join("rslsync_audit_test");
        let _ = fs::remove_dir_all(&tmp);
        let storage = tmp.join("storage");
        let sync_dir = tmp.join("sync");
        fs::create_dir_all(&storage).unwrap();
        fs::create_dir_all(&sync_dir).unwrap();

        let fake_proc = tmp.join("max_user_watches");
        fs::write(&fake_proc, "8192\n").unwrap();
        assert_eq!(read_inotify_max_watches_from(&fake_proc), Some(8192));

        let cfg = ValidatedSyncConf {
            storage_path: storage.to_string_lossy().to_string(),
            pid_file: storage.join("sync.pid").to_string_lossy().to_string(),
            directory_root: sync_dir.to_string_lossy().to_string(),
            listening_port: 55555,
            webui_port: 8888,
        };
        assert!(audit_storage_and_paths(&cfg).is_ok());

        let _ = fs::remove_dir_all(&tmp);
    }
}
