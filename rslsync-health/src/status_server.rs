//! HTTP status server for `:9198/status` and JSON telemetry formatter.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sync_probe::{collect_snapshot, SyncHealthSnapshot};

pub fn format_status_http_response(snap: &SyncHealthSnapshot) -> String {
    let healthy = snap.is_healthy();
    let status_code = if healthy {
        "200 OK"
    } else {
        "503 Service Unavailable"
    };
    let status_label = if healthy { "healthy" } else { "unhealthy" };

    let body = format!(
        "{{\"status\":\"{status_label}\",\"daemon_pid_alive\":{},\"webui_up\":{},\"sync_port_up\":{},\"storage_initialized\":{},\"webui_port\":{},\"sync_port\":{},\"inotify_max_watches\":{},\"uptime_seconds\":{}}}\n",
        snap.daemon_pid_alive,
        snap.webui_up,
        snap.sync_port_up,
        snap.storage_initialized,
        snap.webui_port,
        snap.sync_port,
        snap.inotify_max_watches,
        snap.uptime_seconds
    );

    format!(
        "HTTP/1.1 {status_code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub fn handle_status_client(mut client: TcpStream, start_epoch: u64, sync_conf_path: &Path) {
    let _ = client.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(3)));
    let mut req_buf = [0u8; 1024];
    let _ = client.read(&mut req_buf);

    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let uptime = now_epoch.saturating_sub(start_epoch);

    let snap = collect_snapshot(sync_conf_path, uptime);
    let resp = format_status_http_response(&snap);
    let _ = client.write_all(resp.as_bytes());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_format_status_http_response() {
        let ok_snap = SyncHealthSnapshot {
            daemon_pid_alive: true,
            webui_up: true,
            sync_port_up: true,
            storage_initialized: true,
            webui_port: 8888,
            sync_port: 55555,
            inotify_max_watches: 524288,
            uptime_seconds: 120,
        };
        let resp = format_status_http_response(&ok_snap);
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"status\":\"healthy\""));
        assert!(resp.contains("\"daemon_pid_alive\":true"));

        let down_snap = SyncHealthSnapshot {
            daemon_pid_alive: false,
            webui_up: false,
            sync_port_up: false,
            storage_initialized: false,
            webui_port: 8888,
            sync_port: 55555,
            inotify_max_watches: 8192,
            uptime_seconds: 4,
        };
        let down_resp = format_status_http_response(&down_snap);
        assert!(down_resp.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(down_resp.contains("\"status\":\"unhealthy\""));
    }
}
