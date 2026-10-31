//! Live Resilio Sync v3 port discovery (`http.port` -> `sync.conf`), PID liveness, and TCP/HTTP probe.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct SyncHealthSnapshot {
    pub daemon_pid_alive: bool,
    pub webui_up: bool,
    pub sync_port_up: bool,
    pub storage_initialized: bool,
    pub webui_port: u16,
    pub sync_port: u16,
    pub inotify_max_watches: u64,
    pub uptime_seconds: u64,
}

impl SyncHealthSnapshot {
    pub fn is_healthy(&self) -> bool {
        self.daemon_pid_alive && self.webui_up && self.sync_port_up
    }
}

fn strip_comments_outside_strings(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
        } else if b == b'"' {
            in_str = true;
            out.push(b);
            i += 1;
        } else if (b == b'/' && bytes.get(i + 1) == Some(&b'/')) || b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn extract_sync_conf_ports(raw_conf: &str) -> (String, String, u16, u16) {
    let mut storage_path = "/config/storage".to_string();
    let mut custom_pid_file: Option<String> = None;
    let mut webui_port: u16 = 8888;
    let mut sync_port: u16 = 55555;

    let clean_lines = strip_comments_outside_strings(raw_conf);

    for segment in clean_lines.split([',', '{', '}', '\n']) {
        let trimmed = segment.trim();
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().trim_matches('"');
            let val = v.trim().trim_matches('"').trim();
            if key == "storage_path" && !val.is_empty() {
                storage_path = val.to_string();
            } else if key == "pid_file" && !val.is_empty() {
                custom_pid_file = Some(val.to_string());
            } else if key == "listening_port" {
                if let Ok(p) = val.parse::<u16>() {
                    if p > 0 {
                        sync_port = p;
                    }
                }
            } else if key == "listen" {
                if let Some((_, port_str)) = val.rsplit_once(':') {
                    if let Ok(p) = port_str.parse::<u16>() {
                        if p > 0 {
                            webui_port = p;
                        }
                    }
                }
            }
        }
    }
    let pid_file = custom_pid_file.unwrap_or_else(|| {
        Path::new(&storage_path)
            .join("sync.pid")
            .to_string_lossy()
            .into_owned()
    });
    (storage_path, pid_file, webui_port, sync_port)
}

pub fn resolve_live_ports(sync_conf_path: &Path) -> (String, String, u16, u16) {
    let raw = fs::read_to_string(sync_conf_path).unwrap_or_default();
    let (storage_path, pid_file, conf_webui_port, sync_port) = extract_sync_conf_ports(&raw);

    let http_port_file = Path::new(&storage_path).join("http.port");
    if let Ok(port_raw) = fs::read_to_string(&http_port_file) {
        if let Ok(live_port) = port_raw.trim().parse::<u16>() {
            if live_port > 0 {
                return (storage_path, pid_file, live_port, sync_port);
            }
        }
    }
    (storage_path, pid_file, conf_webui_port, sync_port)
}

pub fn check_pid_alive(storage_path: &str, pid_file_path: &str) -> (bool, bool) {
    let sp = Path::new(storage_path);
    let pid_file = Path::new(pid_file_path);
    let settings_dat = sp.join("settings.dat");
    let storage_initialized = pid_file.exists() || settings_dat.exists();

    let Ok(pid_raw) = fs::read_to_string(pid_file) else {
        return (false, storage_initialized);
    };
    let pid = pid_raw.trim();
    if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
        return (false, storage_initialized);
    }
    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(cmdline) = fs::read(cmdline_path) {
        let text = String::from_utf8_lossy(&cmdline);
        return (text.contains("rslsync"), storage_initialized);
    }
    (false, storage_initialized)
}

pub fn probe_webui_http(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /gui/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 256];
    let Ok(n) = stream.read(&mut buf) else {
        return false;
    };
    let status_line = String::from_utf8_lossy(&buf[..n]);
    status_line.starts_with("HTTP/1.1 200")
        || status_line.starts_with("HTTP/1.1 301")
        || status_line.starts_with("HTTP/1.1 401")
        || status_line.starts_with("HTTP/1.0 200")
}

pub fn probe_tcp_port(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

pub fn collect_snapshot(sync_conf_path: &Path, uptime_seconds: u64) -> SyncHealthSnapshot {
    let (storage_path, pid_file, webui_port, sync_port) = resolve_live_ports(sync_conf_path);
    let (daemon_pid_alive, storage_initialized) = check_pid_alive(&storage_path, &pid_file);
    let webui_up = probe_webui_http(webui_port);
    let sync_port_up = probe_tcp_port(sync_port);
    let inotify_max_watches = fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    SyncHealthSnapshot {
        daemon_pid_alive,
        webui_up,
        sync_port_up,
        storage_initialized,
        webui_port,
        sync_port,
        inotify_max_watches,
        uptime_seconds,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_http_port_overrides_sync_conf() {
        let tmp = std::env::temp_dir().join("rslsync_probe_test");
        let _ = fs::remove_dir_all(&tmp);
        let storage = tmp.join("storage");
        fs::create_dir_all(&storage).unwrap();

        let conf = tmp.join("sync.conf");
        fs::write(
            &conf,
            format!(
                "{{\n  \"storage_path\": \"{}\" // inline comment\n  , \"pid_file\": \"/config/custom.pid\", \"listening_port\": 54321 // inline\n  , \"webui\": {{\"listen\": \"0.0.0.0:8888\"}}\n}}",
                storage.display()
            ),
        )
        .unwrap();

        let (sp, pf, p1, sp1) = resolve_live_ports(&conf);
        assert_eq!(sp, storage.display().to_string());
        assert_eq!(pf, "/config/custom.pid");
        assert_eq!(p1, 8888);
        assert_eq!(sp1, 54321);

        fs::write(storage.join("http.port"), "48888\n").unwrap();
        let (_, _, p2, _) = resolve_live_ports(&conf);
        assert_eq!(p2, 48888);

        let _ = fs::remove_dir_all(&tmp);
    }
}
