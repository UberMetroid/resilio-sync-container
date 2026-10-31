//! Directory provisioning, dynamic `/etc/passwd` synthesis, `sync.conf` rendering, and NAS IgnoreList seeder.

use std::env;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const DEFAULT_SYNC_CONF: &str = include_str!("../../assets/default-sync.conf");
const DEFAULT_IGNORE_LIST: &str = include_str!("../../assets/default-IgnoreList");
const IGNORE_MARKER: &str = "# UBERMETROID_NAS_METADATA_IGNORES";

#[derive(Debug, Clone)]
pub struct SyncEnvConfig {
    pub device_name: String,
    pub sync_port: u16,
    pub webui_port: u16,
    pub health_port: u16,
    pub seed_ignore_list: bool,
}

impl SyncEnvConfig {
    pub fn from_env() -> Self {
        Self {
            device_name: sanitize_device_name(
                &env::var("DEVICE_NAME").unwrap_or_else(|_| "resilio-sync".to_string()),
                "resilio-sync",
            ),
            sync_port: env::var("SYNC_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&p| p >= 1024)
                .unwrap_or(55555),
            webui_port: env::var("WEBUI_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&p| p > 0)
                .unwrap_or(8888),
            health_port: env::var("HEALTH_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&p| p > 0)
                .unwrap_or(9198),
            seed_ignore_list: env::var("SEED_IGNORE_LIST")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
        }
    }
}

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

pub fn synthesize_nss_entries(passwd_path: &Path, group_path: &Path, puid: u32, pgid: u32) {
    let puid_str = puid.to_string();
    let pgid_str = pgid.to_string();
    if let Ok(existing_pw) = fs::read_to_string(passwd_path) {
        let has_uid = existing_pw
            .lines()
            .any(|line| line.split(':').nth(2) == Some(puid_str.as_str()));
        if !has_uid {
            let mut updated = existing_pw;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!(
                "rslsync-{puid}:x:{puid}:{pgid}:Resilio Sync User:/config:/bin/sh\n"
            ));
            if let Err(e) = fs::write(passwd_path, updated) {
                eprintln!(
                    "[rslsync-init] WARN: Failed to write NSS entry to {}: {e}",
                    passwd_path.display()
                );
            }
        }
    }
    if let Ok(existing_gr) = fs::read_to_string(group_path) {
        let has_gid = existing_gr
            .lines()
            .any(|line| line.split(':').nth(2) == Some(pgid_str.as_str()));
        if !has_gid {
            let mut updated = existing_gr;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!("rslsync-{pgid}:x:{pgid}:\n"));
            if let Err(e) = fs::write(group_path, updated) {
                eprintln!(
                    "[rslsync-init] WARN: Failed to write NSS entry to {}: {e}",
                    group_path.display()
                );
            }
        }
    }
}

pub fn seed_folder_ignore_lists(sync_root: &Path) {
    let Ok(entries) = fs::read_dir(sync_root) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        let dot_sync = folder.join(".sync");
        if !fs::symlink_metadata(&dot_sync)
            .map(|m| m.file_type().is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let ignore_file = dot_sync.join("IgnoreList");
        if fs::symlink_metadata(&ignore_file)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        if let Ok(existing) = fs::read_to_string(&ignore_file) {
            if existing.contains(IGNORE_MARKER) {
                continue;
            }
            let mut merged = existing;
            if !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push_str(DEFAULT_IGNORE_LIST);
            let _ = fs::write(&ignore_file, merged);
        } else {
            let _ = fs::write(&ignore_file, DEFAULT_IGNORE_LIST);
        }
    }
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
    if !Path::new("/sync").exists() {
        let _ = fs::create_dir_all("/sync");
        let _ = fs::set_permissions("/sync", Permissions::from_mode(0o777 & !umask_val));
    }
    let _ = fs::set_permissions("/tmp", Permissions::from_mode(0o1777));

    if seed_ignores {
        seed_folder_ignore_lists(Path::new("/sync"));
    }

    if is_root {
        synthesize_nss_entries(
            Path::new("/etc/passwd"),
            Path::new("/etc/group"),
            puid,
            pgid,
        );
        // Strictly chown -R /config only; on /sync ONLY top-level non-recursive chown (never -R!)
        let owner = format!("{puid}:{pgid}");
        let _ = Command::new("chown")
            .args(["-R", &owner, "/config"])
            .status();
        let _ = Command::new("chown").args([&owner, "/sync"]).status();
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

        let pw = tmp.join("passwd");
        let gr = tmp.join("group");
        fs::write(
            &pw,
            "root:x:0:0:root:/root:/bin/sh\nnginx:x:101:99:nginx:/var/lib/nginx:/sbin/nologin\n",
        )
        .unwrap();
        fs::write(&gr, "root:x:0:\n").unwrap();

        synthesize_nss_entries(&pw, &gr, 99, 100);
        assert!(fs::read_to_string(&pw)
            .unwrap()
            .contains("rslsync-99:x:99:100:"));
        assert!(fs::read_to_string(&gr).unwrap().contains(":100:"));

        let share_dot_sync = tmp.join("sync/photos/.sync");
        fs::create_dir_all(&share_dot_sync).unwrap();
        seed_folder_ignore_lists(&tmp.join("sync"));
        let ig = fs::read_to_string(share_dot_sync.join("IgnoreList")).unwrap();
        assert!(ig.contains("@eaDir"));
        assert!(ig.contains(".Recycle.Bin"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
