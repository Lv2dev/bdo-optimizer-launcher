use std::fs;
use std::path::{Path, PathBuf};

fn manifest_declares_dependency(manifest: &str, dependency: &str) -> bool {
    let prefix = format!("{dependency} =");
    manifest
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with(&prefix))
}

fn read_text(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_else(|err| {
        panic!("failed to read {relative}: {err}");
    })
}

fn collect_existing_paths(root: &Path, paths: &[&str]) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|path| root.join(path))
        .filter(|path| path.exists())
        .collect()
}

#[test]
fn tauri_migration_guard_has_no_slint_runtime_residue() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();

    let manifest = read_text(root, "Cargo.toml");
    for dependency in ["slint", "crossbeam-channel", "tray-icon", "image"] {
        if manifest_declares_dependency(&manifest, dependency) {
            failures.push(format!(
                "Cargo.toml still declares dependency `{dependency}`"
            ));
        }
    }

    let obsolete_paths =
        collect_existing_paths(root, &["src/ui", "ui", "scripts/check_button_layout.ps1"]);
    for path in obsolete_paths {
        failures.push(format!(
            "obsolete Slint path still exists: {}",
            path.display()
        ));
    }

    for relative in [
        "README.md",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        "scripts/check_release_workflow.ps1",
    ] {
        let text = read_text(root, relative);
        if text.contains("check_button_layout") || text.contains("Slint UI") {
            failures.push(format!("{relative} still references Slint UI verification"));
        }
    }

    assert!(
        failures.is_empty(),
        "Slint migration residue remains:\n{}",
        failures.join("\n")
    );
}

// 커스텀 타이틀바 드래그/최소화/최대화/닫기는 capabilities/default.json의 core:window 권한에
// 의존한다. 파일이 삭제되거나 권한이 빠지면 런타임에 조용히 동작 불능이 되므로 정적으로 잠근다.
#[test]
fn tauri_capability_grants_window_controls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capability = read_text(root, "capabilities/default.json");
    for required in [
        "\"main\"",
        "core:window:allow-start-dragging",
        "core:window:allow-minimize",
        "core:window:allow-toggle-maximize",
        "core:window:allow-internal-toggle-maximize",
        "core:window:allow-close",
    ] {
        assert!(
            capability.contains(required),
            "capabilities/default.json is missing `{required}`"
        );
    }
}
