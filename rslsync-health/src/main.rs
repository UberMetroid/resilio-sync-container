//! Resilio Sync v3 zero-dependency healthcheck CLI (`--check`) and `:9198/status` server.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod status_server;
mod sync_probe;

use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use status_server::handle_status_client;
use sync_probe::collect_snapshot;

fn log(msg: &str) {
    eprintln!("[rslsync-health] {msg}");
}

fn run_cli_check(bind_port: u16, sync_conf: &Path) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], bind_port));
    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = stream
            .write_all(b"GET /status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        let mut buf = [0u8; 1024];
        if let Ok(n) = stream.read(&mut buf) {
            let reply = String::from_utf8_lossy(&buf[..n]);
            if reply.starts_with("HTTP/1.1 200") {
                return Ok(());
            }
            return Err(format!(
                "Status server returned unhealthy response: {}",
                reply.lines().next().unwrap_or("")
            ));
        }
    }

    let snap = collect_snapshot(sync_conf, 0);
    if snap.is_healthy() {
        Ok(())
    } else {
        Err(format!(
            "Unhealthy snapshot: pid_alive={}, webui_up={}(:{}), sync_port_up={} (:{})",
            snap.daemon_pid_alive,
            snap.webui_up,
            snap.webui_port,
            snap.sync_port_up,
            snap.sync_port
        ))
    }
}

fn main() {
    let bind_port: u16 = env::var("HEALTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9198);
    let bind_host = env::var("HEALTH_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());
    let sync_conf = PathBuf::from(
        env::var("SYNC_CONF_PATH").unwrap_or_else(|_| "/config/sync.conf".to_string()),
    );

    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check") {
        match run_cli_check(bind_port, &sync_conf) {
            Ok(()) => exit(0),
            Err(e) => {
                log(&format!("UNHEALTHY: {e}"));
                exit(1);
            }
        }
    }

    let listen_addr = format!("{bind_host}:{bind_port}");
    let listener = match TcpListener::bind(&listen_addr) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("FATAL: Failed to bind {listen_addr}: {e}"));
            exit(1);
        }
    };

    log(&format!("Listening on http://{listen_addr}/status"));
    let start_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for incoming in listener.incoming().flatten() {
        let conf_clone = sync_conf.clone();
        thread::spawn(move || {
            handle_status_client(incoming, start_epoch, &conf_clone);
        });
    }
}
