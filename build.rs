fn main() {
    // M62-1: Cargo.toml의 버전을 빌드 시 env로 노출한다.
    // Tauri UI도 같은 env 값을 app state로 받아 버전 표기를 동기화한다.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=APP_VERSION={}", version);
    println!("cargo:rerun-if-env-changed=UPDATE_RELEASES_API_URL");
    println!("cargo:rerun-if-env-changed=GITHUB_REPOSITORY");
    let update_url = std::env::var("UPDATE_RELEASES_API_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| {
            std::env::var("GITHUB_REPOSITORY")
                .ok()
                .filter(|repo| !repo.trim().is_empty())
                .map(|repo| format!("https://api.github.com/repos/{repo}/releases/latest"))
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=UPDATE_RELEASES_API_URL={}", update_url);

    let manifest_path = if std::env::var("PROFILE").as_deref() == Ok("release") {
        "app.manifest"
    } else {
        "app.dev.manifest"
    };
    println!("cargo:rerun-if-changed={}", manifest_path);
    println!("cargo:rerun-if-changed=assets/app.ico");

    let manifest = std::fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {manifest_path}: {e}"));
    let windows = tauri_build::WindowsAttributes::new()
        .window_icon_path("assets/app.ico")
        .app_manifest(manifest);
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}
