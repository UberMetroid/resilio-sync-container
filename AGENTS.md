# resilio-sync-container — House Laws & Agent Engineering Standards

This document is the **single canonical source of truth** for all code, architecture, and system integration standards for `resilio-sync-container` (hardened GNU Guix / Nix + zero-dependency Rust Resilio Sync v3 P2P synchronization appliance for **Fedora Server**, **Unraid**, and **Synology NAS**). Every human contributor and AI agent must strictly follow these rules without exception.

**Precedence:** Where this file and CI disagree, this file wins. A rule not enforced anywhere is a bug in this file, not a suggestion to ignore.

**Rule Markers:**
- `[GATE:<tool>]` — Machine-enforced by automated compiler flags, clippy, or `cargo test`. Must pass in CI.
- `[POLICY]` — Enforced during code review. Dictates architecture, naming taste, restraint, and ergonomics.

---

## 1. The Page Rule (Code Layout & Sizing)

A **page** is one committed Rust file. Every page holds one idea, fits in one head, and carries its own weight.

- `[GATE:test]` **16–256 Lines**: Every committed `.rs` file must be between **16 and 256 lines** (`content.lines().count()`, blank lines and comments count). Enforced by `tests/page_rules.rs`.
- `[GATE:test]` **Shim Exemption (Floor Only)**: A file is a shim if every non-empty, non-comment, non-attribute line starts with `mod `, `use `, `pub mod `, or `pub use `. Shims skip the 16-line floor; the 256-line ceiling still strictly applies.
- `[GATE:test]` **Directory Density (≤ 8 files)**: At most 8 `.rs` files per directory. Crowded directories split into functional subdirectories grouped by domain.
- `[GATE:test]` **Banned Drawer Filenames**: `util.rs`, `utils.rs`, `helper.rs`, `helpers.rs`, `common.rs`, `misc.rs`, `shared.rs`, `base.rs`, `core.rs`.
- `[POLICY]` **Domain Naming Recipe**: Name pages after their concrete noun or verb:
  - Action pages: `json_parser.rs`, `validate_sync_conf.rs`, `audit_host_limits.rs`, `spawn_daemon.rs`, `sync_probe.rs`.
  - State pages: `sync_config.rs`, `status_server.rs`.
- `[POLICY]` **Blobs Out**: Move static configurations, ignore lists, and icons into `assets/` and load with `include_str!`.
- `[POLICY]` **Never Pad**: Do not add comments or blank lines to clear 16 lines. Fold small files into their single caller or nearest sibling.

---

## 2. Architecture & Zero-Dependency Discipline

- `[GATE:test]` **Zero External Dependencies**: All 3 workspace crates (`rslsync-init`, `rslsync-config`, `rslsync-health`) must compile with zero external crates in `[dependencies]` (`std` only) for reproducible offline GNU Guix and Nix builds.
- `[POLICY]` **Deep Modules over Speculative Traits (Rule of Two)**: Do not define traits or generic type parameters unless at least two distinct implementations exist in the repository today. Prefer concrete structs with private fields and cohesive public methods.
- `[POLICY]` **Clean Implementation & Attribution**: All Rust supervision code, Guix/Nix manifests, and templates in this repository are original and licensed under the MIT license. The upstream `rslsync` binary is proprietary to Resilio, Inc.

---

## 3. Pure Rust & Crash-Resilience Standards

- `[GATE:clippy]` **Zero-Panic Invariant**: No bare `.unwrap()`, `.expect()`, or `panic!()` in production code (`#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`). `.unwrap()` and `.expect()` are permitted **only** inside `#[cfg(test)]` modules.
- `[GATE:rustc]` **`#![forbid(unsafe_code)]`**: Unsafe code is strictly forbidden across all crate roots and test modules.
- `[GATE:test]` **No Shell Invocations**: Subprocesses must never invoke `sh -c` or `bash -c`. Use direct `std::process::Command::new(argv0).args(...)`. Enforced by `tests/lint.rs`.

---

## 4. Resilio Sync v3 & Multi-NAS Security Invariants

- `[GATE:test]` **Fail-Closed JSON & Schema Validation (`rslsync-config`)**:
  1. Before `rslsync` starts or restarts, `rslsync-config` strips `//` and `/* */` comments outside JSON string literals and parses `/config/sync.conf` with a strict recursive-descent JSON parser (rejecting trailing commas, unquoted keys, duplicate keys, and single quotes).
  2. All top-level keys and `webui` keys are validated against an explicit Resilio Sync v3 key whitelist so unknown keys (which cause `rslsync` to exit immediately) are caught pre-flight.
  3. `listening_port`, `webui.listen` port, and `HEALTH_PORT` (`9198`) must be distinct valid ports, `storage_path` must reside under `/config` and exist on disk, `directory_root` must be `/` or reside under `/users`, `/shared`, `/other`, or `/downloads`, and `files_default_path` (`/downloads`) must reside inside `directory_root` and an allowed data root.
- `[GATE:test]` **Multi-Host Filesystem & ACL Safety (`rslsync-init`)**:
  1. **Never** run `chown -R` or `chmod -R` on `/users`, `/shared`, `/other`, or `/downloads` (preserving Synology Btrfs `synoacl` inheritance and avoiding multi-terabyte Unraid `shfs` stalls). Recursive ownership is applied strictly to `/config`.
  2. Dynamic `/etc/passwd` and `/etc/group` entries are synthesized at runtime for arbitrary host `PUID`/`PGID` (`99:100` on Unraid, `1026:100` on Synology DSM, `1000:1000` on Fedora Server) so glibc `getpwuid()` inside `rslsync` always resolves.
  3. Safe OS/NAS metadata ignore patterns (`@eaDir`, `#recycle`, `#snapshot`, `.SynologyWorkingDirectory`, `.Recycle.Bin`, `.Trash-*`) are opt-in (`SEED_IGNORE_LIST=false` by default; when enabled with `SEED_IGNORE_LIST=true`, seeded once into existing `.sync/IgnoreList` files under `/users`, `/shared`, `/other`, and `/downloads` without touching user files).
  4. On container shutdown, `rslsync-init` sends `SIGTERM` and polls for up to 10 seconds so `rslsync` cleanly checkpoints its SQLite WAL (`storage.db-wal`) before exit.
- `[GATE:test]` **Live Telemetry & Dynamic Port Healthcheck (`rslsync-health`)**:
  `rslsync-health` resolves the active WebUI port from `<storage_path>/http.port` first (where `rslsync` persists live GUI port changes) before falling back to `/config/sync.conf`, verifies `/proc/<pid>/cmdline` from `<storage_path>/sync.pid`, probes `GET /gui/` and the sync TCP port, and exposes JSON telemetry on `:9198/status` and `--check`.

---

## 5. Verification Ladder

Order matters. Each step must pass completely before running the next:

1. `cargo test --workspace --all-targets` — All unit, integration, and Page Rule lint tests pass.
2. `cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic` — Zero warnings, zero panics.
3. `cargo fmt --all -- --check` — Standard formatting verified.
4. `cargo build --release --workspace` — Stripped release binaries built cleanly.

---

## 6. Known Gaps — Not Enforced

- **Proprietary Upstream Binary & EULA**: The `rslsync` binary (`3.1.2.1076-1`) is proprietary software owned by Resilio, Inc., whose Terms of Use (§4b) restrict public binary redistribution. This repository distributes only MIT-licensed Rust source code, Guix/Nix build definitions, and platform templates; CI fetches the SHA256-pinned `.deb` directly from `linux-packages.resilio.com` to build and smoke-test the image.
- **LAN Multicast Peer Discovery**: Local network multicast peer discovery (`239.192.0.0:3838/udp`) cannot traverse Docker/Podman bridge NAT; operators requiring zero-tracker LAN multicast discovery must run with `--network=host` (and optionally set `HEALTH_BIND=127.0.0.1`).
- **Shared Host Kernel `inotify` Watch Budget**: Linux `fs.inotify.max_user_watches` is enforced per real UID across the host kernel (e.g., shared across all containers running as `UID 99` on Unraid, and defaulting to `8192` on Synology DSM). `rslsync-config` and `rslsync-health` report and warn on low host limits but cannot raise kernel sysctls from an unprivileged container.

---

## 7. Multi-Engine Audit Protocol (Pre-Release)

Advisory tiered audit run before tagging releases, budgeted by token capacity:
1. **`mcode`** (Abundant tokens): Full codebase scan for contract mismatches, packaging specs, and CI gates.  
   `mcode exec --permission full "<prompt>"`
2. **`vibe`** (Moderate tokens): Adversarial diff inspection for subtle regressions, security bypasses, and API validity.  
   `vibe -p "<prompt>" --trust --auto-approve`
3. **`grok`** (Strict quota): High-value referee and final sanity check on critical release diffs.  
   `grok -p "<prompt>" --always-approve`
4. **`gemma4`** (Local offline): On-device validation pass with local weights.  
   `gemma4 -p "<prompt>"`

**Rule:** External audits are strictly advisory; all findings must be verified against source code. External LLMs never gate CI—the §5 Verification Ladder is the sole canonical authority.
