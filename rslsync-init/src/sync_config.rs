//! Directory provisioning, dynamic `/etc/passwd` synthesis, `sync.conf` rendering, and NAS IgnoreList seeder.

use std::env;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const DEFAULT_SYNC_CONF: &str = include_str!("../../assets/default-sync.conf");
const DEFAULT_IGNORE_LIST: &str = include_str!("../../assets/default-IgnoreList");

#[derive(Debug, Clone)]
pub struct SyncEnvConfig {
    pub device_name: String,
    pub sync_port: u16,
    pub webui_port: u16,
    pub health_port: u16,
    pub seed_ignore_list: bool,
}

fn env_u16(key: &str, min_port: u16, default_port: u16) -> u16 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&p| p >= min_port)
        .unwrap_or(default_port)
}

impl SyncEnvConfig {
    pub fn from_env() -> Self {
        Self {
            device_name: sanitize_device_name(
                &env::var("DEVICE_NAME").unwrap_or_else(|_| "resilio-sync".to_string()),
                "resilio-sync",
            ),
            sync_port: env_u16("SYNC_PORT", 1024, 55555),
            webui_port: env_u16("WEBUI_PORT", 1, 8888),
            health_port: env_u16("HEALTH_PORT", 1, 9198),
            seed_ignore_list: env::var("SEED_IGNORE_LIST")
                .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
                .unwrap_or(true),
        }
    }
}

pub const DATA_ROOT_DIRS: [&str; 4] = ["/users", "/shared", "/other", "/downloads"];

pub fn sanitize_device_name(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn render_sync_conf(cfg: &SyncEnvConfig) -> String {
    DEFAULT_SYNC_CONF
        .replace("__DEVICE_NAME__", &cfg.device_name)
        .replace("__SYNC_PORT__", &cfg.sync_port.to_string())
        .replace("__WEBUI_PORT__", &cfg.webui_port.to_string())
}

fn append_nss_if_missing(path: &Path, id_str: &str, entry_line: &str) {
    if let Ok(mut existing) = fs::read_to_string(path) {
        if !existing
            .lines()
            .any(|line| line.split(':').nth(2) == Some(id_str))
        {
            if !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(entry_line);
            let _ = fs::write(path, existing);
        }
    }
}

pub fn synthesize_nss_entries(passwd_path: &Path, group_path: &Path, puid: u32, pgid: u32) {
    let pw_line = format!("rslsync-{puid}:x:{puid}:{pgid}:Resilio Sync User:/config:/bin/sh\n");
    append_nss_if_missing(passwd_path, &puid.to_string(), &pw_line);
    append_nss_if_missing(
        group_path,
        &pgid.to_string(),
        &format!("rslsync-{pgid}:x:{pgid}:\n"),
    );
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
}

fn seed_dot_sync_dir(folder: &Path) {
    let dot_sync = folder.join(".sync");
    if !is_real_dir(&dot_sync) {
        return;
    }
    let ignore_file = dot_sync.join("IgnoreList");
    if fs::symlink_metadata(&ignore_file)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return;
    }
    if let Ok(mut merged) = fs::read_to_string(&ignore_file) {
        let mut changed = false;
        for line in DEFAULT_IGNORE_LIST.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !merged.lines().any(|l| l.trim() == trimmed) {
                if !merged.ends_with('\n') && !merged.is_empty() {
                    merged.push('\n');
                }
                merged.push_str(trimmed);
                merged.push('\n');
                changed = true;
            }
        }
        if changed {
            let _ = fs::write(&ignore_file, merged);
        }
    } else {
        let _ = fs::write(&ignore_file, DEFAULT_IGNORE_LIST);
    }
}

fn seed_recursive(dir: &Path, remaining_depth: u8) {
    seed_dot_sync_dir(dir);
    if remaining_depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !matches!(
            entry.file_name().to_string_lossy().as_ref(),
            ".sync" | "@eaDir" | "@tmp" | "#recycle" | "#snapshot"
        ) && is_real_dir(&path)
        {
            seed_recursive(&path, remaining_depth - 1);
        }
    }
}

pub fn seed_folder_ignore_lists(sync_root: &Path) {
    seed_recursive(sync_root, 3);
}

pub fn ensure_directories_and_ownership(
    puid: u32,
    pgid: u32,
    umask_val: u32,
    is_root: bool,
    seed_ignores: bool,
) {
    let dir_mode = 0o2000 | (0o777 & !umask_val);
    for d in ["/config", "/config/storage", "/tmp"] {
        let _ = fs::create_dir_all(d);
        if d != "/tmp" {
            let _ = fs::set_permissions(d, Permissions::from_mode(dir_mode));
        }
    }
    for d in DATA_ROOT_DIRS {
        if !Path::new(d).exists() {
            let _ = fs::create_dir_all(d);
            let _ = fs::set_permissions(d, Permissions::from_mode(0o777 & !umask_val));
        }
        if seed_ignores {
            seed_folder_ignore_lists(Path::new(d));
        }
    }
    let _ = fs::set_permissions("/tmp", Permissions::from_mode(0o1777));

    if is_root {
        synthesize_nss_entries(
            Path::new("/etc/passwd"),
            Path::new("/etc/group"),
            puid,
            pgid,
        );
        // Strictly chown -R /config only; on data roots ONLY top-level non-recursive chown (never -R!)
        let owner = format!("{puid}:{pgid}");
        let _ = Command::new("chown")
            .args(["-R", &owner, "/config"])
            .status();
        for d in DATA_ROOT_DIRS {
            let _ = Command::new("chown").args([&owner, d]).status();
        }
    }
}

pub fn ensure_active_sync_conf(
    conf_path: &Path,
    cfg: &SyncEnvConfig,
    umask_val: u32,
) -> Result<String, String> {
    let file_mode = 0o640 & !umask_val;
    if conf_path.exists() {
        if let Ok(existing) = fs::read_to_string(conf_path) {
            if !existing.trim().is_empty() {
                return Ok(existing);
            }
        }
    }
    let generated = render_sync_conf(cfg);
    fs::write(conf_path, &generated)
        .map_err(|e| format!("Failed writing {}: {e}", conf_path.display()))?;
    let _ = fs::set_permissions(conf_path, Permissions::from_mode(file_mode));
    Ok(generated)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesize_nss_and_seed_ignore_list() {
        let tmp = std::env::temp_dir().join("rslsync_cfg_unit_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let (pw, gr) = (tmp.join("passwd"), tmp.join("group"));
        fs::write(&pw, "root:x:0:0:root:/root:/bin/sh\n").unwrap();
        fs::write(&gr, "root:x:0:\n").unwrap();
        synthesize_nss_entries(&pw, &gr, 99, 100);
        assert!(fs::read_to_string(&pw).unwrap().contains("rslsync-99:"));

        let dirs = [
            tmp.join("users/.sync"),
            tmp.join("users/jeryd/.sync"),
            tmp.join("users/jeryd/Desktop/.sync"),
            tmp.join("users/jeryd/Desktop/proj/.sync"),
        ];
        let d4_sync = tmp.join("users/jeryd/Desktop/proj/deep/.sync");
        for dir in dirs.iter().chain([&d4_sync]) {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(dirs[1].join("IgnoreList"), "#recycle\n").unwrap();
        seed_folder_ignore_lists(&tmp.join("users"));
        for dir in &dirs {
            let ig = fs::read_to_string(dir.join("IgnoreList")).unwrap();
            assert!(ig.contains("?recycle") && ig.contains("@*") && ig.contains("$RECYCLE.BIN"));
        }
        assert!(!d4_sync.join("IgnoreList").exists());
        assert!(SyncEnvConfig::from_env().seed_ignore_list);
        let _ = fs::remove_dir_all(&tmp);
    }
}
