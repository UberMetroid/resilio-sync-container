//! Resilio Sync v3 schema validator: key whitelist, port collision checks, and path confinement.

use crate::json_parser::{parse_sync_conf_json, JsonValue};

#[rustfmt::skip]
const ALLOWED_TOP_KEYS: &[&str] = &[
    "device_name", "listening_port", "storage_path", "pid_file", "use_upnp", "use_gui",
    "download_limit", "upload_limit", "directory_root", "directory_root_policy",
    "files_default_path", "vendor", "display_new_version", "lan_encrypt_data",
    "rate_limit_local_peers", "folder_rescan_interval", "max_file_size_for_versioning",
    "sync_max_time_diff", "sync_trash_ttl", "overwrite_changes", "shared_folders",
    "webui", "proxy_type", "proxy_addr", "proxy_port", "proxy_auth", "proxy_username",
    "proxy_password", "search_lan", "known_hosts", "folder_defaults.delete_to_trash",
    "folder_defaults.use_tracker", "folder_defaults.use_relay", "folder_defaults.use_lan_broadcast",
    "folder_defaults.known_hosts", "folder_defaults.transfer_priority", "folder_defaults.delete_unknown_files",
    "folder_defaults.lan_discovery_mode", "bind_interface", "max_packet_size", "use_relay_server",
    "use_tracker", "disk_low_priority", "profiler_enabled", "tracker_protocols", "log_size",
    "log_ttl", "config_refresh_interval", "config_save_interval", "send_statistics",
    "ignore_symlinks", "lazy_indexing", "prefer_utp2_lan", "net.enable_utp2", "net.udp_ipv4_mtu",
    "use_only_bind_interface", "worker_threads_count", "share_file_ttl", "peer_expiration_days",
    "free_space_warning_threshold",
];

#[rustfmt::skip]
const ALLOWED_WEBUI_KEYS: &[&str] = &[
    "listen", "login", "password", "password_hash", "password_hash_unified",
    "password_hash_salt_unified", "allow_empty_password", "dir_whitelist", "force_https",
    "ssl_certificate", "ssl_private_key", "api_key", "context",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedSyncConf {
    pub storage_path: String,
    pub pid_file: String,
    pub directory_root: String,
    pub listening_port: u16,
    pub webui_port: u16,
}

fn find_key<'a>(entries: &'a [(String, JsonValue)], key: &str) -> Option<&'a JsonValue> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn is_confined_to(path_str: &str, base: &str) -> bool {
    let base_clean = base.trim_end_matches('/');
    let trimmed = path_str.trim_end_matches('/');
    if !path_str.starts_with('/') || trimmed.is_empty() {
        return false;
    }
    if trimmed[1..]
        .split('/')
        .any(|seg| seg.is_empty() || seg == ".." || seg == "." || seg.contains('\0'))
    {
        return false;
    }
    trimmed == base_clean || trimmed.starts_with(&format!("{base_clean}/"))
}

pub fn parse_host_port(addr: &str) -> Result<u16, String> {
    let trimmed = addr.trim();
    let (host, port_str) = trimmed
        .rsplit_once(':')
        .ok_or_else(|| format!("webui.listen '{trimmed}' must be in host:port format"))?;
    if host.is_empty() || (host.contains(':') && !(host.starts_with('[') && host.ends_with(']'))) {
        return Err(format!(
            "Invalid host in webui.listen '{trimmed}' (use [ipv6]:port for IPv6)"
        ));
    }
    let port: u16 = port_str
        .parse()
        .map_err(|_| format!("Invalid port in webui.listen '{trimmed}'"))?;
    if port == 0 {
        return Err("webui.listen port cannot be 0".to_string());
    }
    Ok(port)
}

pub fn validate_sync_conf_str(raw: &str, health_port: u16) -> Result<ValidatedSyncConf, String> {
    let root = parse_sync_conf_json(raw)?;
    let obj = root
        .as_object()
        .ok_or_else(|| "Root of sync.conf must be a JSON object".to_string())?;

    for (k, _) in obj {
        if !ALLOWED_TOP_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "Unknown top-level Resilio Sync v3 key '{k}' (daemon will reject unknown keys)"
            ));
        }
    }

    let listening_port_i64 = find_key(obj, "listening_port")
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| "Missing or non-integer 'listening_port' in sync.conf".to_string())?;
    if !(1024..=65535).contains(&listening_port_i64) {
        return Err(format!(
            "listening_port {listening_port_i64} must be between 1024 and 65535"
        ));
    }
    let listening_port = listening_port_i64 as u16;

    let storage_path = find_key(obj, "storage_path")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Missing or non-string 'storage_path' in sync.conf".to_string())?;
    if !is_confined_to(storage_path, "/config") || storage_path.trim_end_matches('/') == "/config" {
        return Err(format!(
            "storage_path '{storage_path}' must reside under /config (e.g. /config/storage) without '..' traversal"
        ));
    }

    let default_pid = format!("{}/sync.pid", storage_path.trim_end_matches('/'));
    let pid_file = find_key(obj, "pid_file")
        .and_then(JsonValue::as_str)
        .unwrap_or(&default_pid);
    if !is_confined_to(pid_file, "/config")
        || pid_file.trim_end_matches('/') == "/config"
        || pid_file.ends_with('/')
    {
        return Err(format!(
            "pid_file '{pid_file}' must be a file path under /config without '..' traversal"
        ));
    }

    let directory_root = find_key(obj, "directory_root")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Missing or non-string 'directory_root' in sync.conf".to_string())?;
    if !is_confined_to(directory_root, "/sync") {
        return Err(format!(
            "directory_root '{directory_root}' must reside under '/sync' without '..' traversal"
        ));
    }

    if let Some(default_path) = find_key(obj, "files_default_path").and_then(JsonValue::as_str) {
        if !is_confined_to(default_path, directory_root) {
            return Err(format!(
                "files_default_path '{default_path}' lies outside directory_root '{directory_root}'"
            ));
        }
    }

    if let Some(upnp) = find_key(obj, "use_upnp") {
        if upnp.as_bool().is_none() {
            return Err("'use_upnp' must be a JSON boolean".to_string());
        }
    }

    let webui = find_key(obj, "webui")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Missing or non-object 'webui' block in sync.conf".to_string())?;

    for (k, _) in webui {
        if !ALLOWED_WEBUI_KEYS.contains(&k.as_str()) {
            return Err(format!("Unknown webui key 'webui.{k}' in sync.conf"));
        }
    }

    let listen_str = find_key(webui, "listen")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Missing 'webui.listen' address in sync.conf".to_string())?;
    let webui_port = parse_host_port(listen_str)?;

    if webui_port == listening_port {
        return Err(format!(
            "Port collision: webui.listen ({webui_port}) == listening_port ({listening_port})"
        ));
    }
    if webui_port == health_port || listening_port == health_port {
        return Err(format!(
            "Port collision with rslsync-health port ({health_port})"
        ));
    }

    Ok(ValidatedSyncConf {
        storage_path: storage_path.to_string(),
        pid_file: pid_file.to_string(),
        directory_root: directory_root.to_string(),
        listening_port,
        webui_port,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const SAMPLE_CONF: &str = r#"{
        // C++ comment with http://url inside comment
        "device_name": "resilio-node",
        "listening_port": 55555,
        "storage_path": "/config/storage",
        "pid_file": "/config/storage/sync.pid",
        "vendor": "docker",
        "display_new_version": false,
        "use_upnp": false,
        "directory_root_policy": "belowroot",
        "directory_root": "/sync/",
        "files_default_path": "/sync",
        "webui": {
            "listen": "0.0.0.0:8888",
            "allow_empty_password": false
        }
    }"#;

    #[test]
    fn test_valid_sync_conf_passes() {
        let v = validate_sync_conf_str(SAMPLE_CONF, 9198).unwrap();
        assert_eq!(v.listening_port, 55555);
        assert_eq!(v.webui_port, 8888);
        assert_eq!(v.storage_path, "/config/storage");

        let with_proxy = SAMPLE_CONF
            .replace(
                "\"use_upnp\": false,",
                "\"use_upnp\": false, \"use_gui\": true, \"proxy_type\": \"SOCKS5\", \"search_lan\": true,",
            )
            .replace("\"/config/storage\"", "\"/config/custom_storage\"")
            .replace("        \"pid_file\": \"/config/storage/sync.pid\",\n", "");
        let v2 = validate_sync_conf_str(&with_proxy, 9198).unwrap();
        assert_eq!(v2.pid_file, "/config/custom_storage/sync.pid");
    }

    #[test]
    fn test_rejects_trailing_comma_and_unknown_keys() {
        let trailing = r#"{"listening_port": 55555,}"#;
        assert!(validate_sync_conf_str(trailing, 9198).is_err());

        let unknown = SAMPLE_CONF.replace("\"vendor\"", "\"storage_paht\"");
        let err = validate_sync_conf_str(&unknown, 9198).unwrap_err();
        assert!(err.contains("Unknown top-level Resilio Sync v3 key 'storage_paht'"));
    }

    #[test]
    fn test_rejects_lsio_downloads_mismatch_and_port_collision() {
        let lsio_bug = SAMPLE_CONF.replace("\"/sync\"", "\"/downloads\"");
        assert!(validate_sync_conf_str(&lsio_bug, 9198).is_err());
        assert!(validate_sync_conf_str(SAMPLE_CONF, 8888).is_err());
        assert!(parse_host_port("::1").is_err());
        assert!(parse_host_port("0.0.0.0:8888:9999").is_err());
        assert_eq!(parse_host_port("[::]:8888").unwrap(), 8888);

        let traversal = SAMPLE_CONF.replace("\"/config/storage\"", "\"/config/../etc\"");
        assert!(validate_sync_conf_str(&traversal, 9198).is_err());
        let sibling = SAMPLE_CONF.replace("\"/config/storage\"", "\"/configfoo\"");
        assert!(validate_sync_conf_str(&sibling, 9198).is_err());
        let double_slash = SAMPLE_CONF.replace("\"/config/storage\"", "\"/config//storage\"");
        assert!(validate_sync_conf_str(&double_slash, 9198).is_err());
        let leading_zero = SAMPLE_CONF.replace("55555", "05555");
        assert!(validate_sync_conf_str(&leading_zero, 9198).is_err());
        let unicode_esc = SAMPLE_CONF.replace("\"resilio-node\"", "\"resilio-\\u0033\"");
        assert!(validate_sync_conf_str(&unicode_esc, 9198).is_ok());
    }
}
