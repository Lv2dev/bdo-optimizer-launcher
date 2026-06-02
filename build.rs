fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile Slint UI");

    // M62-1: Cargo.toml의 버전을 빌드 시 env로 노출 → main.rs에서 Slint property로 전파.
    // settings.slint의 하드코딩된 버전 표기를 단일 진실의 근거(Cargo.toml)와 동기화.
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

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        let manifest = if std::env::var("PROFILE").as_deref() == Ok("release") {
            "app.manifest"
        } else {
            "app.dev.manifest"
        };
        res.set_manifest_file(manifest);
        // 멀티 해상도 아이콘. .ai/scripts/generate_icon.ps1로 생성.
        res.set_icon("assets/app.ico");
        res.compile().expect("failed to embed Windows resources");
    }
}
