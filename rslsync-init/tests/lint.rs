//! Workspace-wide lint tests ensuring zero shell invocations ("sh -c", "bash -c", "zsh -c")
//! and validating Guix manifest, Nix flake, Unraid template, Synology Compose, and Fedora Quadlet.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name != "target" && name != ".git" && name != "prebuilt" {
                collect_rs_files(&path, files);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

#[test]
fn test_no_shell_invocations_in_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .ok_or("Could not find workspace root")?;

    let mut rs_files = Vec::new();
    collect_rs_files(workspace_root, &mut rs_files);
    assert!(!rs_files.is_empty(), "No .rs files found to lint");

    for file in rs_files {
        if file.ends_with("tests/lint.rs") {
            continue;
        }
        let content = fs::read_to_string(&file)?;

        let forbidden_patterns = [
            ("Command::new(", "\"sh\")"),
            ("Command::new(", "\"/bin/sh\")"),
            ("Command::new(", "\"bash\")"),
            ("Command::new(", "\"/bin/bash\")"),
            ("Command::new(", "\"zsh\")"),
            ("Command::new(", "\"/bin/zsh\")"),
        ];
        for (prefix, suffix) in forbidden_patterns {
            let pat = format!("{prefix}{suffix}");
            assert!(
                !content.contains(&pat),
                "Forbidden shell invocation {pat} detected in {}",
                file.display()
            );
        }

        let has_dash_c = content.contains("\"-c\"");
        let has_shell_bin = content.contains("\"sh\"")
            || content.contains("\"bash\"")
            || content.contains("\"zsh\"");
        assert!(
            !(has_dash_c && has_shell_bin),
            "Forbidden shell -c invocation detected in {}",
            file.display()
        );
    }
    Ok(())
}

#[test]
fn test_manifests_and_multi_platform_templates() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .ok_or("Could not find workspace root")?;

    let guix_scm = fs::read_to_string(workspace_root.join("manifest.scm"))?;
    for item in [
        "rslsync-init",
        "rslsync-config",
        "rslsync-health",
        "rslsync",
        "patchelf",
        "libxcrypt",
    ] {
        assert!(
            guix_scm.contains(item),
            "manifest.scm missing required token {item}"
        );
    }

    let flake_nix = fs::read_to_string(workspace_root.join("flake.nix"))?;
    assert!(flake_nix.contains("dontStrip = true"));
    assert!(flake_nix.contains("libxcrypt-legacy"));
    assert!(flake_nix.contains("31b623897fef0682b97f6274d124a7d7003c5d0db47f3fe3407a5c45258fbcbb"));

    let template_xml = fs::read_to_string(workspace_root.join("unraid-template.xml"))?;
    assert!(template_xml.contains("<Name>resilio-sync-container</Name>"));
    assert!(template_xml.contains("assets/resilio-sync-icon.png"));
    assert!(template_xml.contains("9198"));

    let synology_yml = fs::read_to_string(workspace_root.join("assets/compose.synology.yaml"))?;
    assert!(synology_yml.contains("PUID=1026"));
    assert!(synology_yml.contains("PGID=100"));
    assert!(synology_yml.contains("127.0.0.1:9198:9198/tcp"));

    let fedora_quadlet = fs::read_to_string(workspace_root.join("assets/resilio-sync.container"))?;
    assert!(fedora_quadlet.contains("UserNS=keep-id:uid=1000,gid=1000"));
    assert!(fedora_quadlet.contains(":Z"));
    assert!(fedora_quadlet.contains(":z"));

    let ci_yml =
        fs::read_to_string(workspace_root.join(".github/workflows/build-and-publish.yml"))?;
    assert!(ci_yml.contains("docker/login-action@v3"));
    assert!(ci_yml.contains("libcrypt1"));
    assert!(!ci_yml.contains("libxcrypt1"));
    assert!(ci_yml.contains("docker push ghcr.io/ubermetroid/resilio-sync-container:latest"));
    assert!(ci_yml.contains("docker push ghcr.io/ubermetroid/resilio-sync-container:v0.2.0"));

    let readme = fs::read_to_string(workspace_root.join("README.md"))?;
    let docs_html = fs::read_to_string(workspace_root.join("docs/index.html"))?;
    assert!(workspace_root.join("docs/.nojekyll").exists());
    assert!(workspace_root.join("docs/resilio-sync-icon.png").exists());

    for doc in [&readme, &docs_html] {
        let lower = doc.to_ascii_lowercase();
        assert!(
            !lower.contains("zero trust") && !lower.contains("zero-trust"),
            "Documentation must not contain the phrase 'zero trust'"
        );
        assert!(
            !lower.contains("first principles") && !lower.contains("first-principles"),
            "Documentation must not contain the phrase 'first principles'"
        );
        for required in [
            "3.1.2.1076",
            "http.port",
            "compose.synology.yaml",
            "resilio-sync.container",
            "unraid-template.xml",
            ".btskey",
            "9198",
            "SEED_IGNORE_LIST=true",
            "31b623897fef0682b97f6274d124a7d7003c5d0db47f3fe3407a5c45258fbcbb",
        ] {
            assert!(
                doc.contains(required),
                "Documentation missing required user-facing topic: {required}"
            );
        }
    }

    Ok(())
}
