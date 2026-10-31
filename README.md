# Resilio Sync v3 Container (`UberMetroid/resilio-sync-container`)

<p align="center">
  <img src="assets/resilio-sync-icon.png" alt="Resilio Sync v3" width="160" />
</p>

<p align="center">
  <a href="https://github.com/UberMetroid/resilio-sync-container/actions/workflows/build-and-publish.yml"><img src="https://github.com/UberMetroid/resilio-sync-container/actions/workflows/build-and-publish.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/UberMetroid/resilio-sync-container/releases/tag/v0.3.3"><img src="https://img.shields.io/badge/release-v0.3.3-50fa7b?style=flat-square" alt="Release v0.3.3" /></a>
  <a href="https://github.com/UberMetroid/resilio-sync-container/pkgs/container/resilio-sync-container"><img src="https://img.shields.io/badge/rslsync-3.1.2.1076--1-00b4d8?style=flat-square" alt="Resilio Sync 3.1.2.1076-1" /></a>
  <a href="AGENTS.md"><img src="https://img.shields.io/badge/dependencies-zero%20(std%20only)-3fb950?style=flat-square" alt="Zero Dependencies" /></a>
  <a href="AGENTS.md"><img src="https://img.shields.io/badge/unsafe-forbidden-f85149?style=flat-square" alt="No Unsafe" /></a>
  <a href="https://ubermetroid.github.io/resilio-sync-container/"><img src="https://img.shields.io/badge/docs-GitHub%20Pages-3fb950?style=flat-square" alt="GitHub Pages" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-58a6ff?style=flat-square" alt="License: MIT" /></a>
</p>

<p align="center">
  <strong><a href="https://ubermetroid.github.io/resilio-sync-container/">Documentation &amp; Landing Page</a></strong> &middot;
  <strong><a href="https://github.com/UberMetroid/resilio-sync-container/pkgs/container/resilio-sync-container">GHCR Image</a></strong> &middot;
  <strong><a href="https://github.com/UberMetroid/resilio-sync-container/releases">Releases</a></strong> &middot;
  <strong><a href="AGENTS.md">House Laws (AGENTS.md)</a></strong>
</p>

**Resilio Sync v3 (`3.1.2.1076-1`)** container appliance for **Unraid**, **Synology NAS (DSM 7.2+)**, **Fedora Server**, and standard **Docker / Podman** hosts. Built with **GNU Guix / Nix** and three static, pure-`std` Rust supervision binaries (`rslsync-init`, `rslsync-config`, `rslsync-health`) with top-level `/config`, `/users`, `/shared`, `/other`, and `/downloads` mounts that keep your NAS permissions intact, validate `/config/sync.conf` before launch, and track live WebUI port changes without killing the daemon.

---

## Why a Dedicated Resilio Sync v3 Container?

In late 2024, Resilio made **Resilio Sync v3** (`3.x`) completely free for personal and commercial use, removing the old Pro vs. Home feature split (selective sync, custom folder permissions, and multi-folder synchronization are now included for everyone). However, running v3 in a container hits four practical problems:

1. **The official `resilio/sync` Docker image stopped at `v2.8.1`.** Resilio no longer updates their Docker Hub image and distributes v3 (`3.1.2.1076-1`) exclusively as `.deb` and `.rpm` system packages.
2. **Changing the WebUI port in the GUI kills existing third-party containers.** When you change the WebUI port inside Resilio's Preferences UI, `rslsync` writes the new active port to `/config/storage/http.port` and does **not** rewrite `/config/sync.conf`. Images that hardcode `nc -z localhost 8888` in their Docker `HEALTHCHECK` mark the container unhealthy and enter an infinite restart loop.
3. **Mismatched default folder paths break WebUI folder creation.** Third-party images like `linuxserver/resilio-sync` ship a default `sync.conf` with `"directory_root": "/sync/"` alongside `"files_default_path": "/downloads"`. Because `/downloads` lies outside `/sync/`, the WebUI folder picker errors out unless you manually edit JSON before starting.
4. **Recursive `chown -R` on data volumes destroys NAS ACLs and stalls boot.** Running `chown -R` or `chmod -R` across data shares at container startup strips Synology DSM Btrfs extended ACLs (`synoacl`) and triggers multi-minute spin-up stalls across multi-terabyte Unraid `shfs` arrays.

### How This Image Fixes Each Failure Mode

| Failure Mode | What `resilio-sync-container` Does | Enforced By |
| :--- | :--- | :--- |
| **GUI WebUI port changes** | Reads `/config/storage/http.port` first before falling back to `/config/sync.conf`, verifies `/proc/<pid>/cmdline` against `sync.pid`, and probes `GET /gui/` on the live port. | `rslsync-health` |
| **Broken `sync.conf` edits or typos** | Strips `//` and `/* */` comments outside JSON string literals, parses `/config/sync.conf` with a strict recursive-descent JSON parser, checks every key against the Resilio Sync v3 key whitelist, verifies `files_default_path` (`/downloads`) resides inside `directory_root` (`/`) and allowed data roots (`/users`, `/shared`, `/other`, `/downloads`), and checks for port collisions across `webui.listen`, `listening_port`, and `HEALTH_PORT` (`9198`). If a live edit is invalid, `rslsync-init` logs the exact error and keeps the running daemon alive. | `rslsync-config` + `rslsync-init` |
| **NAS ACL destruction & array stalls** | Applies recursive ownership (`chown -R`) strictly to `/config`. **Never runs `chown -R` or `chmod -R` on `/users`, `/shared`, `/other`, or `/downloads`**, preserving Synology `synoacl` inheritance and Unraid `shfs` responsiveness. | `rslsync-init` |
| **Arbitrary host `PUID`/`PGID` crashes** | Synthesizes matching `/etc/passwd` and `/etc/group` entries at startup for any runtime `PUID`/`PGID` (`99:100` Unraid, `1026:100` Synology, `1000:1000` Fedora) so glibc `getpwuid()` inside `rslsync` always resolves cleanly. | `rslsync-init` |
| **NAS Recycle Bin & `@eaDir` synced across peers** | Enabled by default (`SEED_IGNORE_LIST=true`; set `SEED_IGNORE_LIST=false` to disable): seeds `?recycle`, `?snapshot`, `?SynoRecycle` (using `?` wildcard because `#` at the start of a line is a comment in `.sync/IgnoreList`), `@eaDir`, `@tmp`, `@SynoResource`, `@*`, `.SynologyWorkingDirectory`, `.Recycle.Bin`, `$RECYCLE.BIN`, `.fuse_hidden*`, `.Trash-*`, `.DS_Store`, and `Thumbs.db` into existing `.sync/IgnoreList` files at root and up to depth 3 under `/users`, `/shared`, `/other`, and `/downloads`. | `rslsync-init` |
| **SQLite database corruption on stop** | Intercepts container stop requests and gives `rslsync` a 10-second `SIGTERM` window to checkpoint `/config/storage/storage.db-wal` cleanly before exit. | `rslsync-init` |
| **ELF stripping breaks embedded WebUI** | Preserves the 17.4 MB WebUI ZIP archive appended to the `rslsync` ELF binary (`dontStrip = true` in Nix; `patchelf --set-interpreter` / `--set-rpath` only in Guix). | `manifest.scm` & `flake.nix` |

---

## 60-Second Setup by Platform

### Option A: Unraid (WebUI Template)

Download the Unraid template directly to your flash drive:

```bash
curl -fsSL https://raw.githubusercontent.com/UberMetroid/resilio-sync-container/main/unraid-template.xml \
  -o /boot/config/plugins/dockerMan/templates-user/my-resilio-sync.xml
```

1. In the Unraid WebUI, go to **Docker** &rarr; **Add Container** &rarr; select **`resilio-sync-container`** under **User Templates**.
2. Verify your top-level paths:
   - **AppData Config Path (`/config`):** `/mnt/user/appdata/resilio-sync/config` (or `/mnt/cache/appdata/resilio-sync/config` to keep SQLite writes off the parity array).
   - **Users Share Path (`/users`):** `/mnt/user/users`
   - **Shared Share Path (`/shared`):** `/mnt/user/shared`
   - **Other Share Path (`/other`):** `/mnt/user/other`
   - **Downloads Share Path (`/downloads`):** `/mnt/user/downloads`
3. Click **Apply**.
   - Pre-configured with Unraid's native `PUID=99` (`nobody`), `PGID=100` (`users`), and `UMASK=002` (`775` directories / `664` files) so SMB shares and Resilio Sync coexist without permission conflicts.
   - Health telemetry binds to port `9198` (`http://<unraid-ip>:9198/status`), avoiding port collisions alongside `manta` (`:9199`), `barracuda` (`:9299`), and `urchin` (`:9399`).

---

### Option B: Synology NAS (DSM 7.2+ Container Manager)

#### Step 1: Raise Synology's `inotify` Watch Limit (One-Time Setup)
Synology DSM defaults `fs.inotify.max_user_watches` to `8192`. Once your synced folders pass a few thousand files, real-time change detection stops working unless this limit is raised.
1. Open DSM **Control Panel** &rarr; **Task Scheduler**.
2. Click **Create** &rarr; **Triggered Task** &rarr; **User-defined script**.
   - **Task:** `Raise inotify watches`
   - **User:** `root`
   - **Event:** `Boot-up`
3. Under **Task Settings** &rarr; **Run command**, paste:
   ```bash
   sysctl -w fs.inotify.max_user_watches=524288
   ```
4. Click **OK**, then right-click the task and select **Run** once to apply it immediately.

#### Step 2: Deploy in Container Manager
1. Create the persistent directories on your volume (`/volume1/docker/resilio-sync/config`, `/volume1/users`, `/volume1/shared`, `/volume1/other`, and `/volume1/downloads`) and grant Read/Write access to your admin user (`UID 1026`, `GID 100`) in DSM **File Station**.
2. Open **Container Manager** &rarr; **Project** &rarr; **Create**, and paste [`assets/compose.synology.yaml`](assets/compose.synology.yaml):

```yaml
services:
  resilio-sync:
    image: ghcr.io/ubermetroid/resilio-sync-container:latest
    container_name: resilio-sync
    hostname: synology-resilio-sync
    restart: unless-stopped
    entrypoint: ["/bin/rslsync-init"]
    security_opt:
      - no-new-privileges:true
    environment:
      - PUID=1026
      - PGID=100
      - UMASK=002
      - DEVICE_NAME=synology-resilio-sync
      - WEBUI_PORT=8888
      - SYNC_PORT=55555
      - HEALTH_PORT=9198
      - SEED_IGNORE_LIST=true
      - DROP_PRIVS=true
    ports:
      - "8888:8888/tcp"
      - "55555:55555/tcp"
      - "55555:55555/udp"
      - "127.0.0.1:9198:9198/tcp"
    volumes:
      - /volume1/docker/resilio-sync/config:/config:rw
      - /volume1/users:/users:rw
      - /volume1/shared:/shared:rw
      - /volume1/other:/other:rw
      - /volume1/downloads:/downloads:rw
    healthcheck:
      test: ["CMD", "/bin/rslsync-health", "--check"]
      interval: 30s
      timeout: 5s
      retries: 3
```

Because `rslsync-init` never runs `chown -R` or `chmod -R` on `/users`, `/shared`, `/other`, or `/downloads`, your Synology Btrfs `synoacl` permissions remain untouched. With `SEED_IGNORE_LIST=true` (the default), `?recycle`, `?snapshot`, `?SynoRecycle`, `@eaDir`, `@tmp`, `@*`, `.Recycle.Bin`, and `$RECYCLE.BIN` are automatically seeded into `.sync/IgnoreList` across top-level and nested shares.

---

### Option C: Fedora Server (Podman Quadlet + SELinux)

[`assets/resilio-sync.container`](assets/resilio-sync.container) is a native `systemd` Podman Quadlet unit configured with:
- `UserNS=keep-id:uid=1000,gid=1000` so container `UID 1000` maps cleanly to your Fedora user on disk.
- `:Z` (private SELinux label on `/config`) and `:z` (shared SELinux label on `/users`, `/shared`, `/other`, and `/downloads` so Samba, NFS, or other containers can share the volumes under `SELinux Enforcing`).
- `ExecStop=/usr/bin/podman exec resilio-sync /bin/rslsync-init --shutdown` for a clean 10-second SQLite WAL checkpoint on service stop.

```bash
# 1. Create host directories
sudo mkdir -p /var/lib/resilio-sync/config /srv/resilio-sync/{users,shared,other,downloads}
sudo chown 1000:1000 /var/lib/resilio-sync/config /srv/resilio-sync/{users,shared,other,downloads}

# 2. Install the Quadlet unit and start the service
sudo cp assets/resilio-sync.container /etc/containers/systemd/
sudo systemctl daemon-reload
sudo systemctl start resilio-sync

# 3. Open WebUI and P2P sync ports in firewalld
sudo firewall-cmd --permanent --add-port=8888/tcp --add-port=55555/tcp --add-port=55555/udp
sudo firewall-cmd --reload
```

*(For rootless Podman, copy `assets/resilio-sync.container` to `~/.config/containers/systemd/` and run `systemctl --user daemon-reload && systemctl --user start resilio-sync`.)*

---

### Option D: Docker / Podman CLI

```bash
docker run -d \
  --name=resilio-sync \
  --restart=unless-stopped \
  --security-opt=no-new-privileges:true \
  --entrypoint=/bin/rslsync-init \
  -p 8888:8888/tcp \
  -p 55555:55555/tcp \
  -p 55555:55555/udp \
  -p 127.0.0.1:9198:9198/tcp \
  -e PUID=1000 \
  -e PGID=1000 \
  -e UMASK=002 \
  -e DEVICE_NAME=resilio-sync \
  -v /srv/resilio-sync/config:/config:rw \
  -v /srv/resilio-sync/users:/users:rw \
  -v /srv/resilio-sync/shared:/shared:rw \
  -v /srv/resilio-sync/other:/other:rw \
  -v /srv/resilio-sync/downloads:/downloads:rw \
  ghcr.io/ubermetroid/resilio-sync-container:latest
```

---

## First-Run WebUI Setup & Applying Your Free v3 License Key

1. **Open the WebUI:** Navigate to `http://<host-ip>:8888/gui/` in your browser.
2. **Create your WebUI admin account:** On first launch, Resilio Sync prompts you to set a local administrator username and password (`allow_empty_password` is set to `false` in `/config/sync.conf`).
3. **Register for a free Resilio Sync v3 license key (`.btskey`):**
   - Resilio Sync v3 requires a free `.btskey` license file activated through the WebUI.
   - Request your free key from Resilio's official v3 registration page (or locate your existing `.btskey` file).
   - In the WebUI, click the **Gear icon (Preferences)** in the top-right corner &rarr; **License** &rarr; **Apply license** and select or upload your `.btskey` file (you can also place your `.btskey` file inside `/config` on the host and browse to `/config` directly in the WebUI).
4. **License persistence:** Your identity, peer certificates, folder database, and applied license are stored inside `/config/storage` (`settings.dat` and `storage.db`). As long as `/config` is mounted to persistent host storage, container upgrades and restarts preserve your license and peer identity automatically.
5. **Adding folders:** The WebUI folder picker is restricted via `webui.dir_whitelist` (`["/config", "/users", "/shared", "/other", "/downloads"]`) to `/config`, `/users`, `/shared`, `/other`, and `/downloads`, with `"files_default_path": "/downloads"` as the default folder destination.

---

## Networking: LAN Discovery, Bridge vs. Host Mode & Port Forwarding

Understanding how Resilio Sync discovers peers and transfers data saves hours of troubleshooting slow transfers:

| Traffic Type | Port & Protocol | Bridge Mode (`-p 55555:55555`) | Host Mode (`--network=host`) | What You Need to Know |
| :--- | :--- | :--- | :--- | :--- |
| **Direct P2P Data Transfer** | `55555/tcp` & `55555/udp` | Works at full wire speed | Works at full wire speed | Always publish **both TCP and UDP** (`µTP` uses UDP for congestion-friendly high-speed transfers). Forward `55555/tcp` and `55555/udp` on your router for remote WAN peers to connect directly instead of falling back to slow relay servers. |
| **Local LAN Multicast Discovery** | `239.192.0.0:3838/udp` | Blocked by Docker/Podman bridge NAT | Works natively on local subnet | Multicast packets (`239.192.0.0:3838/udp`) do not cross container bridge NAT. See the two options below for air-gapped or zero-tracker LAN sync. |
| **WebUI Administration** | `8888/tcp` (or live `http.port`) | Published via `-p 8888:8888/tcp` | Bound directly on host | Do **not** expose port `8888` directly to the public internet; access it over LAN, Tailscale/WireGuard, or an authenticated reverse proxy. |
| **Health & Telemetry API** | `9198/tcp` | Published via `-p 9198:9198/tcp` | Bound directly on host | Set `HEALTH_BIND=127.0.0.1` when running with `--network=host` if you only want local health queries. |

### How to Get Full LAN Speeds in Bridge Mode (Without `--network=host`)
If you keep the container in standard `bridge` network mode (recommended for port isolation):
1. **Tracker / Relay-Assisted Handshake:** If your peers have internet access, Resilio's tracker exchanges each peer's local RFC1918 LAN IP (`192.168.x.x:55555`), allowing peers on the same LAN to connect directly to `<nas-lan-ip>:55555` over TCP/UDP at full gigabit/10GbE speeds.
2. **Predefined Hosts (Air-Gapped / Trackerless LAN):** In the WebUI, right-click a synced folder &rarr; **Preferences** &rarr; check **Use predefined hosts** &rarr; add your NAS LAN IP and port (`192.168.1.50:55555`). Or run with `--network=host` (and `-e HEALTH_BIND=127.0.0.1`) so `239.192.0.0:3838/udp` multicast works automatically on your LAN.

---

## Directory Layout, Environment Variables & Live Config Editing

### Container Paths

| Container Path | Purpose | Ownership Behavior on Startup |
| :--- | :--- | :--- |
| `/config` | Holds `/config/sync.conf`, `.btskey` license files (`webui.dir_whitelist`), and `/config/storage/` (`settings.dat`, `storage.db`, `sync.pid`, `http.port`) | Created if missing; `chown -R PUID:PGID /config` is applied so `rslsync` can always read/write its SQLite state. |
| `/users` | Top-level user home/profile sync folders (`webui.dir_whitelist`) | Created if missing; top-level `chown PUID:PGID /users` only. **Never touched by `chown -R` or `chmod -R`.** |
| `/shared` | Top-level shared team/family folders (`webui.dir_whitelist`) | Created if missing; top-level `chown PUID:PGID /shared` only. **Never touched by `chown -R` or `chmod -R`.** |
| `/other` | Top-level miscellaneous sync folders (`webui.dir_whitelist`) | Created if missing; top-level `chown PUID:PGID /other` only. **Never touched by `chown -R` or `chmod -R`.** |
| `/downloads` | Top-level downloads & default sync folder path (`files_default_path`, `webui.dir_whitelist`) | Created if missing; top-level `chown PUID:PGID /downloads` only. **Never touched by `chown -R` or `chmod -R`.** |

### Environment Variables

| Variable | Default | Purpose |
| :--- | :---: | :--- |
| `PUID` | `99` (root) or effective UID | Runtime user ID for `rslsync` and `rslsync-health` (`99` on Unraid, `1026` on Synology DSM, `1000` on Fedora Server). |
| `PGID` | `100` | Runtime group ID (`100` on Unraid/Synology, `1000` on Fedora Server). |
| `UMASK` | `002` | Octal file creation mask (`002` creates `775` directories and `664` files for group-writable NAS shares). |
| `DEVICE_NAME` | `resilio-sync` | Peer device name rendered into `/config/sync.conf` on first boot (sanitized to `[A-Za-z0-9._-]`, max 64 chars). |
| `WEBUI_PORT` | `8888` | Initial WebUI listening port rendered into `/config/sync.conf` on first boot. |
| `SYNC_PORT` | `55555` | P2P TCP/UDP listening port rendered into `/config/sync.conf` (`1024..=65535`). |
| `HEALTH_PORT` | `9198` | HTTP listening port for `rslsync-health` (`GET /status`). Must not collide with `WEBUI_PORT` or `SYNC_PORT`. |
| `HEALTH_BIND` | `0.0.0.0` | Bind address for `rslsync-health` (set to `127.0.0.1` for loopback-only access in `--network=host` mode). |
| `SEED_IGNORE_LIST` | `true` | Enabled by default (`SEED_IGNORE_LIST=true`; set `SEED_IGNORE_LIST=false` to disable): seeds NAS/OS metadata exclusions (`?recycle`, `?snapshot`, `?SynoRecycle`, `@eaDir`, `@tmp`, `@*`, `.Recycle.Bin`, `$RECYCLE.BIN`, etc.) into existing `.sync/IgnoreList` files at root and up to depth 3 under `/users`, `/shared`, `/other`, and `/downloads`. |
| `DROP_PRIVS` | `true` | Drop privileges from `root` to `PUID:PGID` via `setpriv --reuid --regid --clear-groups` before spawning `rslsync`. |
| `SYNC_CONF_PATH` | `/config/sync.conf` | Path to the active Resilio Sync JSON configuration file. |

### NAS Recycle Bin & Synology `@` Folder Exclusion Rules (`.sync/IgnoreList`)

> **Why `?recycle` is required instead of `#recycle`:** Resilio Sync treats any line starting with `#` in `.sync/IgnoreList` as a **comment**. Writing `#recycle`, `#snapshot`, or `#SynoRecycle` at the start of a line causes Resilio Sync to ignore the rule as a comment and sync your Synology Recycle Bin across all peers! Using `?recycle`, `?snapshot`, and `?SynoRecycle` (where the `?` single-character wildcard matches `#` without starting the line with `#`) together with `@*` (`@eaDir`, `@tmp`, `@SynoResource`), `.Recycle.Bin`, and `$RECYCLE.BIN` properly blocks Recycle Bins and Synology system directories.

When `SEED_IGNORE_LIST=true` (the default), `rslsync-init` automatically seeds [`assets/default-IgnoreList`](assets/default-IgnoreList) into every existing `.sync/IgnoreList` at each share root (`/users/.sync`, `/shared/.sync`, `/other/.sync`, `/downloads/.sync`) and up to 3 directory levels deep (e.g., `/users/<user>/.sync` and `/users/<user>/Desktop/.sync`):

```gitignore
# UBERMETROID_NAS_METADATA_IGNORES
# Note: Resilio Sync treats lines starting with '#' as comments, so '?recycle',
# '?snapshot', and '?SynoRecycle' use '?' wildcard to match '#' at line start.
?recycle
?snapshot
?SynoRecycle
#recycle
#snapshot
#SynoRecycle
@eaDir
@tmp
@SynoResource
@*
.SynologyWorkingDirectory
.Recycle.Bin
$RECYCLE.BIN
$Recycle.Bin
.fuse_hidden*
.Trash-*
.DS_Store
.Spotlight-V100
.Trashes
ehthumbs.db
Thumbs.db
desktop.ini
```

### Live `/config/sync.conf` Editing & Hot Reload
`rslsync-init` only generates `/config/sync.conf` if it does not already exist (or is empty). Thereafter, your custom edits to `/config/sync.conf` are preserved across restarts.
- Every 5 seconds, `rslsync-init` checks the modification time (`mtime`) of `/config/sync.conf`.
- When you save a change to `/config/sync.conf`, `rslsync-init` runs `/bin/rslsync-config --config /config/sync.conf` before touching the running daemon:
  - **If valid:** `rslsync-init` sends `SIGTERM` to `rslsync`, waits for SQLite WAL checkpointing, and restarts `rslsync` with the updated configuration.
  - **If invalid** (syntax error, trailing comma, unknown key typo, or port collision): `rslsync-init` logs the validation error and **keeps your existing `rslsync` process running uninterrupted**.

---

## Health Checks & JSON Telemetry (`:9198/status`)

Query `http://<host-ip>:9198/status` for real-time container and daemon health (returns `HTTP 200 OK` when healthy, `HTTP 503 Service Unavailable` when unhealthy):

```bash
curl -s http://localhost:9198/status
```

```json
{
  "status": "healthy",
  "daemon_pid_alive": true,
  "webui_up": true,
  "sync_port_up": true,
  "storage_initialized": true,
  "webui_port": 8888,
  "sync_port": 55555,
  "inotify_max_watches": 524288,
  "uptime_seconds": 86420
}
```

Inside the container (or in Docker/Podman `HEALTHCHECK`), run:

```bash
docker exec resilio-sync /bin/rslsync-health --check
```

And to validate a configuration file manually before applying it:

```bash
docker exec resilio-sync /bin/rslsync-config --config /config/sync.conf
```

---

## Build & Verify It Yourself (Rust + GNU Guix / Nix)

All three supervision binaries (`rslsync-init`, `rslsync-config`, `rslsync-health`) compile with **zero external crates** (`std` only, `Cargo.lock` contains only the 3 workspace members), `#![forbid(unsafe_code)]`, `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, and zero shell (`sh -c` / `bash -c`) invocations.

### 1. Run the Verification Suite

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
cargo fmt --all -- --check
```

### 2. Build the Container with Nix (`flake.nix`)

`flake.nix` fetches the SHA256-pinned Resilio Sync v3 `.deb` (`3.1.2.1076-1`, `amd64` sha256 `31b623897fef0682b97f6274d124a7d7003c5d0db47f3fe3407a5c45258fbcbb`, `arm64` sha256 `2db3a56568d3a89dc8a60d9ec36c19b1a911787e34356ec091561dc6cd3f4463`), compiles the Rust workspace offline, and builds a reproducible layered OCI image:

```bash
nix build .#default
docker load < result
```

### 3. Or Build the Container with GNU Guix (`manifest.scm`)

```bash
# 1. Build static Rust supervision binaries
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C target-cpu=x86-64" cargo build --release --workspace --target x86_64-unknown-linux-musl
mkdir -p prebuilt
cp target/x86_64-unknown-linux-musl/release/rslsync-init prebuilt/
cp target/x86_64-unknown-linux-musl/release/rslsync-config prebuilt/
cp target/x86_64-unknown-linux-musl/release/rslsync-health prebuilt/

# 2. Fetch and verify the SHA256-pinned upstream Resilio Sync v3 .deb
curl -fsSL "https://linux-packages.resilio.com/resilio-sync/deb/pool/non-free/r/resilio-sync/resilio-sync_3.1.2.1076-1_amd64.deb" \
  -o /tmp/resilio-sync.deb
echo "31b623897fef0682b97f6274d124a7d7003c5d0db47f3fe3407a5c45258fbcbb  /tmp/resilio-sync.deb" | sha256sum -c -
ar x /tmp/resilio-sync.deb && tar -xf data.tar.*
cp usr/bin/rslsync prebuilt/rslsync

# 3. Pack the reproducible container tarball with GNU Guix
guix pack -f docker \
  --entry-point=/bin/rslsync-init \
  -S /bin=bin -S /etc=etc -S /share=share \
  -m manifest.scm
```

---

## License & Upstream Attribution

- **Supervision Suite, Manifests & Templates (`rslsync-init`, `rslsync-config`, `rslsync-health`, `manifest.scm`, `flake.nix`, `assets/`):** Licensed under the [MIT License](LICENSE).
- **Resilio Sync v3 (`rslsync` `3.1.2.1076-1`):** Proprietary software owned by Resilio, Inc. Fetched from `linux-packages.resilio.com` and verified by SHA256 digest during image build.
