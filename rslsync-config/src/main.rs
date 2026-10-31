//! Fail-closed Resilio Sync v3 JSON config & host environment validator CLI (`rslsync-config`).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod audit_host_limits;
mod json_parser;
mod validate_sync_conf;

use std::env;
use std::fs;
use std::process::exit;

use audit_host_limits::{
    audit_storage_and_paths, check_inotify_watch_budget, RECOMMENDED_MIN_INOTIFY_WATCHES,
};
use validate_sync_conf::validate_sync_conf_str;

fn log(msg: &str) {
    eprintln!("[rslsync-config] {msg}");
}

fn parse_args() -> (String, bool) {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config_path = "/config/sync.conf".to_string();
    let mut skip_fs = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_path.clone_from(&args[i + 1]);
            i += 2;
        } else if args[i] == "--skip-fs-checks" {
            skip_fs = true;
            i += 1;
        } else {
            i += 1;
        }
    }
    (config_path, skip_fs)
}

fn main() {
    let (config_path, skip_fs) = parse_args();
    let health_port: u16 = env::var("HEALTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&p| p > 0)
        .unwrap_or(9198);

    let raw = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("ERROR: Cannot read {config_path}: {e}"));
            exit(1);
        }
    };

    let validated = match validate_sync_conf_str(&raw, health_port) {
        Ok(v) => v,
        Err(e) => {
            log(&format!("ERROR: Invalid {config_path}: {e}"));
            exit(1);
        }
    };

    if !skip_fs {
        if let Err(e) = audit_storage_and_paths(&validated) {
            log(&format!("ERROR: Filesystem check failed: {e}"));
            exit(1);
        }
    }

    if let Some((limit, sufficient)) = check_inotify_watch_budget() {
        if !sufficient {
            log(&format!(
                "WARN: Host fs.inotify.max_user_watches={limit} is below recommended {RECOMMENDED_MIN_INOTIFY_WATCHES} (note: shared per-UID across containers; on Synology DSM / Unraid / Fedora run: sysctl -w fs.inotify.max_user_watches=524288)"
            ));
        }
    }

    log(&format!(
        "OK: Validated {config_path} (webui_port={}, sync_port={}, storage={})",
        validated.webui_port, validated.listening_port, validated.storage_path
    ));
}
