;; GNU Guix reproducible package & manifest definition for resilio-sync-container
;; Build Docker image:
;;   guix pack -f docker --entry-point=/bin/rslsync-init -S /bin=bin -S /etc=etc -S /share=share -m manifest.scm

(use-modules (guix packages)
             (guix gexp)
             (guix build-system gnu)
             (guix build-system trivial)
             ((guix licenses) #:prefix license:)
             (gnu packages linux)
             (gnu packages elf)
             (gnu packages curl)
             (gnu packages certs)
             (gnu packages base)
             (gnu packages bash)
             (gnu packages crypto)
             (gnu packages rust)
             (gnu packages commencement))

;; 1. Source-built package definition (compiles all 3 Rust crates from source inside Guix sandbox)
(define-public rslsync-from-source
  (package
    (name "rslsync-from-source")
    (version "0.3.1")
    (source (local-file "." "resilio-sync-container-checkout"
                        #:recursive? #t
                        #:select? (lambda (file stat)
                                    (not (or (string-contains file "/target")
                                             (string-contains file "/prebuilt")
                                             (string-contains file "/.git"))))))
    (build-system gnu-build-system)
    (native-inputs
     (list rust
           (list rust "cargo")
           gcc-toolchain))
    (arguments
     (list
      #:tests? #f
      #:phases
      #~(modify-phases %standard-phases
          (delete 'configure)
          (replace 'build
            (lambda _
              (setenv "HOME" "/tmp")
              (invoke "cargo" "build" "--release" "--offline")))
          (replace 'install
            (lambda* (#:key outputs #:allow-other-keys)
              (let* ((out (assoc-ref outputs "out"))
                     (bin (string-append out "/bin"))
                     (etc (string-append out "/etc"))
                     (rsl-etc (string-append etc "/resilio-sync")))
                (mkdir-p bin)
                (mkdir-p rsl-etc)
                (for-each
                 (lambda (prog)
                   (install-file (string-append "target/release/" prog) bin))
                 '("rslsync-init"
                   "rslsync-config"
                   "rslsync-health"))
                (install-file "assets/default-sync.conf" rsl-etc)
                (install-file "assets/default-IgnoreList" rsl-etc)))))))
    (home-page "https://github.com/UberMetroid/resilio-sync-container")
    (synopsis "Zero-dependency Rust Resilio Sync v3 supervisor and JSON validator")
    (description
     "Hardened Resilio Sync v3 P2P file synchronization appliance for Fedora Server, Unraid, and Synology NAS built with GNU Guix and 3 zero-dependency Rust binaries.")
    (license license:expat)))

;; 2. Fast packaging of the 3 statically-linked Rust binaries + patchelf-linked rslsync v3 + /etc configuration
(define-public rslsync-suite
  (package
    (name "rslsync-suite")
    (version "3.1.2.1076")
    (source (local-file "." "resilio-sync-container-static"
                        #:recursive? #t
                        #:select? (lambda (file stat)
                                    (or (string=? file ".")
                                        (string-contains file "/prebuilt")
                                        (string-contains file "/assets")))))
    (build-system trivial-build-system)
    (native-inputs (list coreutils patchelf))
    (inputs (list glibc libxcrypt))
    (arguments
     (list
      #:modules '((guix build utils))
      #:builder
      #~(begin
          (use-modules (guix build utils))
          (let* ((src #$source)
                 (out #$output)
                 (bin (string-append out "/bin"))
                 (etc (string-append out "/etc"))
                 (rsl-etc (string-append etc "/resilio-sync"))
                 (patchelf-bin (string-append #$patchelf "/bin/patchelf"))
                 (ld-so (string-append #$glibc "/lib/ld-linux-x86-64.so.2"))
                 (rpath (string-append #$glibc "/lib:" #$libxcrypt "/lib")))
            (mkdir-p bin)
            (mkdir-p rsl-etc)
            (for-each
             (lambda (prog)
               (let ((src-bin (string-append src "/prebuilt/" prog))
                     (dst-bin (string-append bin "/" prog)))
                 (copy-file src-bin dst-bin)
                 (chmod dst-bin #o755)))
             '("rslsync-init"
               "rslsync-config"
               "rslsync-health"
               "rslsync"))
            ;; Patch rslsync ELF interpreter and RPATH without stripping appended WebUI ZIP archive
            (invoke patchelf-bin "--set-interpreter" ld-so
                    "--set-rpath" rpath
                    (string-append bin "/rslsync"))
            (copy-file (string-append src "/assets/default-sync.conf")
                       (string-append rsl-etc "/default-sync.conf"))
            (copy-file (string-append src "/assets/default-IgnoreList")
                       (string-append rsl-etc "/default-IgnoreList"))
            (call-with-output-file (string-append etc "/passwd")
              (lambda (port)
                (display "root:x:0:0:root:/root:/bin/sh\nnobody:x:99:100:Unraid Nobody:/config:/bin/sh\nrslsync:x:1026:100:Synology User:/config:/bin/sh\nfedora:x:1000:1000:Fedora User:/config:/bin/sh\n" port)))
            (call-with-output-file (string-append etc "/group")
              (lambda (port)
                (display "root:x:0:\nusers:x:100:nobody,rslsync\nfedora:x:1000:fedora\n" port)))
            (call-with-output-file (string-append etc "/nsswitch.conf")
              (lambda (port)
                (display "passwd: files\ngroup: files\nhosts: files dns\n" port)))))))
    (home-page "https://github.com/UberMetroid/resilio-sync-container")
    (synopsis "Zero-dependency static Rust Resilio Sync v3 supervisor and multi-NAS appliance")
    (description
     "Hardened Resilio Sync v3 P2P file synchronization appliance built with GNU Guix, Resilio Sync v3.1.2.1076, and 3 zero-dependency static Rust binaries (rslsync-init, rslsync-config, rslsync-health).")
    (license license:expat)))

(concatenate-manifests
 (list (packages->manifest (list rslsync-suite))
       (specifications->manifest
        '("glibc"
          "libxcrypt"
          "util-linux"
          "curl"
          "nss-certs"
          "coreutils"
          "bash-minimal"))))
