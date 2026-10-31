//! Resilio Sync v3 PID 1 container supervisor, NAS provisioner, and live config watchdog.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod spawn_daemon;
mod sync_config;

use std::env;
use std::path::Path;
use std::process::{exit, Child};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use spawn_daemon::{
    file_mtime, find_rslsync_bin, format_umask, parse_octal_umask, read_effective_uid,
    run_validator, spawn_daemon_process, terminate_children, DaemonContext,
};
use sync_config::{ensure_active_sync_conf, ensure_directories_and_ownership, SyncEnvConfig};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

fn log(msg: &str) {
    eprintln!("[rslsync-init] {msg}");
}

fn spawn_or_exit(res: Result<Child, String>) -> Child {
    match res {
        Ok(c) => c,
        Err(e) => {
            log(&format!("FATAL: {e}"));
            exit(1);
        }
    }
}

fn supervise_loop(ctx: &DaemonContext<'_>, mut sync_child: Child, mut health_child: Child) {
    let rslsync_bin = find_rslsync_bin();
    let conf_path = Path::new(ctx.sync_conf_path);
    let mut last_mtime = file_mtime(conf_path);
    let mut last_watch = Instant::now();

    while !SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        if Path::new("/run/rslsync.shutdown").exists() {
            SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
            break;
        }

        if let Ok(Some(st)) = sync_child.try_wait() {
            log(&format!("WARN: rslsync exited with {st}; restarting..."));
            ensure_directories_and_ownership(
                ctx.puid,
                ctx.pgid,
                ctx.umask_val,
                ctx.is_root,
                ctx.seed_ignores,
            );
            if let Ok(c) = spawn_daemon_process(
                rslsync_bin,
                &["--nodaemon", "--config", ctx.sync_conf_path],
                ctx.puid,
                ctx.pgid,
                ctx.umask_val,
                ctx.is_root,
                ctx.drop_privs,
            ) {
                sync_child = c;
            }
        }

        if let Ok(Some(st)) = health_child.try_wait() {
            log(&format!(
                "WARN: rslsync-health exited with {st}; restarting..."
            ));
            if let Ok(c) = spawn_daemon_process(
                "/bin/rslsync-health",
                &[],
                ctx.puid,
                ctx.pgid,
                ctx.umask_val,
                ctx.is_root,
                ctx.drop_privs,
            ) {
                health_child = c;
            }
        }

        if last_watch.elapsed() >= Duration::from_secs(5) {
            last_watch = Instant::now();
            let current_mtime = file_mtime(conf_path);
            if current_mtime.is_some() && current_mtime != last_mtime {
                log("Detected sync.conf modification; validating before restarting rslsync...");
                if run_validator(ctx.sync_conf_path) {
                    let mut single = [sync_child];
                    terminate_children(&mut single);
                    if let Ok(c) = spawn_daemon_process(
                        rslsync_bin,
                        &["--nodaemon", "--config", ctx.sync_conf_path],
                        ctx.puid,
                        ctx.pgid,
                        ctx.umask_val,
                        ctx.is_root,
                        ctx.drop_privs,
                    ) {
                        sync_child = c;
                        log("Restarted rslsync with updated sync.conf.");
                    } else {
                        log("FATAL: Failed to respawn rslsync after config change.");
                        exit(1);
                    }
                } else {
                    log("ERROR: Updated sync.conf failed rslsync-config validation; keeping running instance!");
                }
                last_mtime = current_mtime;
            }
        }

        thread::sleep(Duration::from_millis(500));
    }

    log("Shutdown requested; sending SIGTERM to rslsync (10s WAL checkpoint window)...");
    let mut children = [sync_child, health_child];
    terminate_children(&mut children);
    let _ = std::fs::remove_file("/run/rslsync.shutdown");
    log("Clean shutdown complete.");
}

fn run_shutdown_cli() -> ! {
    let _ = std::fs::create_dir_all("/run");
    if let Err(e) = std::fs::write("/run/rslsync.shutdown", b"1\n") {
        log(&format!("ERROR: Failed writing /run/rslsync.shutdown: {e}"));
        exit(1);
    }
    for _ in 0..24 {
        if !Path::new("/run/rslsync.shutdown").exists() {
            exit(0);
        }
        thread::sleep(Duration::from_millis(500));
    }
    log("ERROR: Timed out waiting 12s for supervisor to acknowledge /run/rslsync.shutdown.");
    exit(1);
}

fn main() {
    if env::args().any(|a| a == "--shutdown") {
        run_shutdown_cli();
    }
    let _ = std::fs::remove_file("/run/rslsync.shutdown");

    let euid = read_effective_uid();
    let is_root = euid == 0;
    let puid: u32 = env::var("PUID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if is_root { 99 } else { euid });
    let pgid: u32 = env::var("PGID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let umask_str = env::var("UMASK").unwrap_or_else(|_| "002".to_string());
    let umask_val = parse_octal_umask(&umask_str);
    let drop_privs = env::var("DROP_PRIVS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let sync_conf_path =
        env::var("SYNC_CONF_PATH").unwrap_or_else(|_| "/config/sync.conf".to_string());

    let cfg = SyncEnvConfig::from_env();
    log(&format!(
        "Starting rslsync-init supervisor (euid={euid}, PUID={puid}, PGID={pgid}, UMASK={})",
        format_umask(umask_val)
    ));

    ensure_directories_and_ownership(puid, pgid, umask_val, is_root, cfg.seed_ignore_list);

    if let Err(e) = ensure_active_sync_conf(Path::new(&sync_conf_path), &cfg, umask_val) {
        log(&format!("FATAL: {e}"));
        exit(1);
    }

    // Re-ensure /config ownership after writing sync.conf
    ensure_directories_and_ownership(puid, pgid, umask_val, is_root, false);

    log("Running rslsync-config pre-flight JSON & v3 schema validation...");
    if !run_validator(&sync_conf_path) {
        log("FATAL: rslsync-config validation failed. Refusing to start rslsync (fail-closed).");
        exit(1);
    }

    let rslsync_bin = find_rslsync_bin();
    let sync_child = spawn_or_exit(spawn_daemon_process(
        rslsync_bin,
        &["--nodaemon", "--config", &sync_conf_path],
        puid,
        pgid,
        umask_val,
        is_root,
        drop_privs,
    ));
    let health_child = spawn_or_exit(spawn_daemon_process(
        "/bin/rslsync-health",
        &[],
        puid,
        pgid,
        umask_val,
        is_root,
        drop_privs,
    ));

    log(&format!(
        "Launched rslsync (webui={}, sync={}) and rslsync-health (:{}/status).",
        cfg.webui_port, cfg.sync_port, cfg.health_port
    ));

    let ctx = DaemonContext {
        puid,
        pgid,
        umask_val,
        is_root,
        drop_privs,
        seed_ignores: cfg.seed_ignore_list,
        sync_conf_path: &sync_conf_path,
    };
    supervise_loop(&ctx, sync_child, health_child);
}
