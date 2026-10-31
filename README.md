# Resilio Sync v3 (`UberMetroid/resilio-sync-container`)

<p align="center">
  <img src="assets/resilio-sync-icon.png" alt="Resilio Sync v3" width="160" />
</p>

Hardened, reproducible **GNU Guix / Nix** + **zero-dependency pure-Rust** container appliance for **Resilio Sync v3 (`3.1.2.1076-1`)**, engineered for **Fedora Server**, **Unraid**, and **Synology NAS**:

1. **Why this exists:** Resilio abandoned their official Docker Hub image (`resilio/sync`) on v2.8.1 and only distributes v3 as `.deb` / `.rpm` packages. Meanwhile, third-party images like `linuxserver/resilio-sync` rely on `s6-overlay` root initialization, ship a broken `"files_default_path": "/downloads"` vs. `"directory_root": "/sync/"` configuration, and hardcode `nc -z localhost 8888` in their healthcheck loop (which kills `rslsync` if you change the WebUI port in the GUI).
2. **Fail-Closed JSON & v3 Key-Whitelist Validation (`rslsync-config`):** Before `rslsync` starts or restarts, `rslsync-config` strips `//` and `/* */` comments outside JSON string literals, parses `/config/sync.conf` with a strict recursive-descent JSON parser, enforces an explicit Resilio Sync v3 key whitelist (catching typos that cause `rslsync` to exit on launch), checks port collisions across `webui.listen`, `listening_port`, and `HEALTH_PORT` (`9198`), and audits host `fs.inotify.max_user_watches`.
3. **Live `http.port` Health Probe (`rslsync-health`):** When you change the WebUI port inside Resilio's GUI, `rslsync` writes the active port to `/config/storage/http.port` without modifying `sync.conf`. `rslsync-health` reads `http.port` first before falling back to `sync.conf`, verifies `/proc/<pid>/cmdline` from `/config/storage/sync.pid`, probes `GET /gui/`, and exposes JSON telemetry on `:9198/status` and `/bin/rslsync-health --check`.
4. **Multi-NAS Filesystem & ACL Safety (`rslsync-init`):**
   - **Never runs `chown -R` or `chmod -R` on `/sync`**, preserving Synology DSM Btrfs extended ACLs (`synoacl`) and avoiding multi-terabyte Unraid `shfs` stalls.
   - Dynamically synthesizes `/etc/passwd` and `/etc/group` entries at container startup for arbitrary host `PUID`/`PGID` (`99:100` on Unraid, `1026:100` on Synology DSM, `1000:1000` on Fedora Server) so glibc `getpwuid()` inside `rslsync` always succeeds.
   - Seeds safe OS/NAS metadata exclusions (`@eaDir`, `#recycle`, `#snapshot`, `.SynologyWorkingDirectory`, `.Recycle.Bin`, `.Trash-*`) into `/sync/*/.sync/IgnoreList`.
   - Enforces a 10-second `SIGTERM` SQLite WAL (`storage.db-wal`) checkpoint window before container stop.

---

## 1. Install by Platform

### Option A: Unraid (WebUI Template)
```bash
curl -fsSL https://raw.githubusercontent.com/UberMetroid/resilio-sync-container/main/unraid-template.xml \
  -o /boot/config/plugins/dockerMan/templates-user/my-resilio-sync.xml
```
In Unraid: **Docker** -> **Add Container** -> select **`resilio-sync-container`** -> **Apply**. Pre-configured with `PUID=99`, `PGID=100`, `UMASK=002`, and health port `9198` (collision-free alongside `manta` `:9199`, `barracuda` `:9299`, and `urchin` `:9399`).

### Option B: Synology NAS (DSM 7.2+ Container Manager)
1. In DSM **Control Panel** -> **Task Scheduler**, create a **Triggered Task** (`Boot-up`, user `root`) to raise Synology's low `8192` inotify limit:
   ```bash
   sysctl -w fs.inotify.max_user_watches=524288
   ```
2. Import [`assets/compose.synology.yaml`](assets/compose.synology.yaml) in **Container Manager** -> **Project** (pre-configured with `PUID=1026`, `PGID=100`, and `@eaDir` / `#recycle` / `#snapshot` ignore protection).

### Option C: Fedora Server (Podman Quadlet + SELinux)
Copy [`assets/resilio-sync.container`](assets/resilio-sync.container) to `/etc/containers/systemd/` (or `~/.config/containers/systemd/` for rootless Podman with `UserNS=keep-id:uid=1000,gid=1000` and `:Z` / `:z` SELinux volume labels):
```bash
sudo cp assets/resilio-sync.container /etc/containers/systemd/
sudo systemctl daemon-reload
sudo systemctl start resilio-sync
```

---

## 2. Ports & Telemetry

| Port | Protocol | Service | Purpose |
| :--- | :---: | :--- | :--- |
| **`8888`** | TCP | **WebUI (`/gui/`)** | Resilio Sync v3 Web administration interface (`http://<host-ip>:8888/gui/`) |
| **`55555`** | TCP + UDP | **P2P Sync Listener** | High-speed TCP and UDP (`µTP`) peer-to-peer synchronization traffic |
| **`9198`** | TCP | **Health & Status API** | `curl http://<host-ip>:9198/status` returns JSON (`healthy`, `daemon_pid_alive`, `webui_up`, `sync_port_up`, `inotify_max_watches`) |

---

## 3. Configuration (`Environment Variables`)

| Variable | Default | Purpose |
| :--- | :---: | :--- |
| `PUID` / `PGID` | `99` / `100` | Unprivileged runtime UID & GID (`99:100` Unraid, `1026:100` Synology, `1000:1000` Fedora) |
| `UMASK` | `002` | Directory and config creation mask (`002` = `775` dirs / `664` files) |
| `DEVICE_NAME` | `resilio-sync` | Peer device name rendered into `/config/sync.conf` |
| `WEBUI_PORT` | `8888` | Initial WebUI port rendered into `/config/sync.conf` |
| `SYNC_PORT` | `55555` | P2P TCP/UDP listening port (`1024..=65535`) |
| `HEALTH_PORT` | `9198` | Health telemetry HTTP server port (`:9198/status`) |
| `HEALTH_BIND` | `0.0.0.0` | Health server bind address (set to `127.0.0.1` when using `--network=host`) |
| `SEED_IGNORE_LIST` | `true` | Auto-seed OS/NAS metadata exclusions (`@eaDir`, `#recycle`, `.Recycle.Bin`) into `/sync/*/.sync/IgnoreList` |
| `DROP_PRIVS` | `true` | Drop privileges from `root` to `PUID:PGID` via `setpriv` |

---

## 4. Build & Verify from Source (Guix or Nix)

```bash
# 1. Run the Rust Verification Ladder
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
cargo fmt --all -- --check

# 2. Build via Nix Flake (dockerTools.buildLayeredImage)
nix build .#default
podman load < result

# 3. Or build via GNU Guix (manifest.scm)
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --workspace --target x86_64-unknown-linux-gnu
mkdir -p prebuilt && cp target/x86_64-unknown-linux-gnu/release/rslsync-* prebuilt/
guix pack -f docker --entry-point=/bin/rslsync-init -S /bin=bin -S /etc=etc -S /share=share -m manifest.scm
```
