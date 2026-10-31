{
  description = "Hardened Nix OCI container for Resilio Sync v3 with zero-dependency Rust supervision (rslsync-init, rslsync-config, rslsync-health)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
          };

          debMeta = {
            "x86_64-linux" = {
              arch = "amd64";
              sha256 = "31b623897fef0682b97f6274d124a7d7003c5d0db47f3fe3407a5c45258fbcbb";
            };
            "aarch64-linux" = {
              arch = "arm64";
              sha256 = "2db3a56568d3a89dc8a60d9ec36c19b1a911787e34356ec091561dc6cd3f4463";
            };
          }.${system};

          rslsync-v3 = pkgs.stdenv.mkDerivation rec {
            pname = "resilio-sync";
            version = "3.1.2.1076";

            src = pkgs.fetchurl {
              url = "https://linux-packages.resilio.com/resilio-sync/deb/pool/non-free/r/resilio-sync/resilio-sync_${version}-1_${debMeta.arch}.deb";
              inherit (debMeta) sha256;
            };

            dontStrip = true; # Preserves appended 17.4MB WebUI ZIP archive inside rslsync binary
            nativeBuildInputs = [
              pkgs.autoPatchelfHook
              pkgs.binutils-unwrapped
            ];
            buildInputs = [
              pkgs.stdenv.cc.libc
              pkgs.libxcrypt-legacy
            ];

            unpackPhase = ''
              ar x $src
              tar -xf data.tar.*
            '';

            installPhase = ''
              runHook preInstall
              install -Dm755 usr/bin/rslsync $out/bin/rslsync
              runHook postInstall
            '';
          };

          rslsync-suite = pkgs.rustPlatform.buildRustPackage {
            pname = "rslsync-suite";
            version = "0.3.3";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            postInstall = ''
              mkdir -p $out/etc/resilio-sync
              cp assets/default-sync.conf $out/etc/resilio-sync/default-sync.conf
              cp assets/default-IgnoreList $out/etc/resilio-sync/default-IgnoreList
            '';
          };
        in {
          inherit rslsync-v3 rslsync-suite;

          default = pkgs.dockerTools.buildLayeredImage {
            name = "ghcr.io/ubermetroid/resilio-sync-container";
            tag = "latest";

            contents = [
              pkgs.bashInteractive
              pkgs.coreutils
              pkgs.util-linux
              pkgs.cacert
              pkgs.tzdata
              pkgs.dockerTools.fakeNss
              rslsync-v3
              rslsync-suite
            ];

            extraCommands = ''
              mkdir -p config config/storage users shared other downloads tmp etc
              chmod 1777 tmp
              for f in etc/passwd etc/group; do
                if [ -L "$f" ]; then
                  cp --remove-destination "$(readlink -f "$f")" "$f"
                fi
              done
              grep -q ':1000:' etc/group || echo 'fedora:x:1000:' >> etc/group
              grep -q ':1000:' etc/passwd || echo 'fedora:x:1000:1000:Fedora User:/config:/bin/sh' >> etc/passwd
              grep -q ':1026:' etc/passwd || echo 'rslsync:x:1026:100:Synology User:/config:/bin/sh' >> etc/passwd
              chmod u+w etc/passwd etc/group || true
            '';

            config = {
              Entrypoint = [ "/bin/rslsync-init" ];
              Labels = {
                "org.opencontainers.image.source" = "https://github.com/UberMetroid/resilio-sync-container";
              };
              Healthcheck = {
                Test = [ "CMD" "/bin/rslsync-health" "--check" ];
                Interval = 30000000000;
                Timeout = 5000000000;
                Retries = 3;
              };
              ExposedPorts = {
                "8888/tcp" = {};
                "9198/tcp" = {};
                "55555/tcp" = {};
                "55555/udp" = {};
              };
              Volumes = {
                "/config" = {};
                "/users" = {};
                "/shared" = {};
                "/other" = {};
                "/downloads" = {};
              };
              Env = [
                "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "TZDIR=${pkgs.tzdata}/share/zoneinfo"
              ];
            };
          };
        });
    };
}
