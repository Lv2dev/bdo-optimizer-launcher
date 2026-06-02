use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestRelease {
    pub tag_name: String,
    pub html_url: String,
    pub exe_asset_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub status_text: String,
    pub release_url: String,
    pub latest_version: String,
    pub update_available: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("업데이트 채널이 설정되지 않았습니다.")]
    ChannelNotConfigured,
    #[error("업데이트 확인 실패: {0}")]
    Http(String),
    #[error("업데이트 정보 해석 실패: {0}")]
    Json(#[from] serde_json::Error),
    #[error("업데이트 버전 형식을 해석할 수 없습니다: {0}")]
    InvalidVersion(String),
    #[error("릴리스 페이지 URL이 올바르지 않습니다.")]
    InvalidReleaseUrl,
    #[error("브라우저 실행 실패: {0}")]
    ExplorerSpawn(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub fn release_api_url(
    explicit_url: Option<&str>,
    github_repository: Option<&str>,
) -> Option<String> {
    if let Some(url) = explicit_url {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_string());
    }
    github_repository
        .map(str::trim)
        .filter(|repo| !repo.is_empty())
        .map(|repo| format!("https://api.github.com/repos/{repo}/releases/latest"))
}

pub fn configured_release_api_url() -> Option<String> {
    release_api_url(option_env!("UPDATE_RELEASES_API_URL"), None)
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let base = version.trim().trim_start_matches('v').split('-').next()?;
    let mut parts = base.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn compare_release_versions(latest: &str, current: &str) -> Option<bool> {
    let latest = parse_version(latest)?;
    let current = parse_version(current)?;
    Some(latest > current)
}

pub fn parse_latest_release_json(json: &str) -> Result<LatestRelease, Error> {
    let release: GitHubRelease = serde_json::from_str(json)?;
    let exe_asset_url = release
        .assets
        .into_iter()
        .find(|asset| {
            asset
                .name
                .eq_ignore_ascii_case("bdo-optimizer-launcher.exe")
        })
        .map(|asset| asset.browser_download_url);
    Ok(LatestRelease {
        tag_name: release.tag_name,
        html_url: release.html_url,
        exe_asset_url,
    })
}

pub fn evaluate_release(
    current_version: &str,
    release: LatestRelease,
) -> Result<UpdateCheck, Error> {
    let update_available = compare_release_versions(&release.tag_name, current_version)
        .ok_or_else(|| Error::InvalidVersion(release.tag_name.clone()))?;
    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let status_text = if update_available {
        format!("새 버전 {latest_version} 사용 가능.")
    } else {
        format!("최신 버전입니다. ({current_version})")
    };
    Ok(UpdateCheck {
        status_text,
        release_url: release.html_url,
        latest_version,
        update_available,
    })
}

fn fetch_latest_release_json(url: &str) -> Result<String, Error> {
    let response = ureq::get(url)
        .set("User-Agent", "bdo-optimizer-launcher")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;
    response
        .into_string()
        .map_err(|e| Error::Http(e.to_string()))
}

pub fn check_latest_release() -> Result<UpdateCheck, Error> {
    let url = configured_release_api_url().ok_or(Error::ChannelNotConfigured)?;
    let json = fetch_latest_release_json(&url)?;
    let release = parse_latest_release_json(&json)?;
    evaluate_release(env!("APP_VERSION"), release)
}

pub fn open_release_page(url: &str) -> Result<(), Error> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") {
        return Err(Error::InvalidReleaseUrl);
    }
    std::process::Command::new(super::windows_path("explorer.exe"))
        .arg(trimmed)
        .spawn()
        .map(|_| ())
        .map_err(Error::ExplorerSpawn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_api_url_prefers_explicit_url_then_github_repository() {
        assert_eq!(
            release_api_url(
                Some("https://api.github.com/repos/a/b/releases/latest"),
                None
            ),
            Some("https://api.github.com/repos/a/b/releases/latest".to_string())
        );
        assert_eq!(
            release_api_url(None, Some("owner/repo")),
            Some("https://api.github.com/repos/owner/repo/releases/latest".to_string())
        );
        assert_eq!(release_api_url(None, None), None);
        assert_eq!(release_api_url(Some("   "), Some("owner/repo")), None);
    }

    #[test]
    fn semver_comparison_accepts_v_prefix_and_prerelease_suffix() {
        assert_eq!(compare_release_versions("v0.2.0", "0.1.0"), Some(true));
        assert_eq!(compare_release_versions("v0.1.0", "0.1.0"), Some(false));
        assert_eq!(
            compare_release_versions("v0.1.0-beta.1", "0.1.0"),
            Some(false)
        );
        assert_eq!(compare_release_versions("not-a-version", "0.1.0"), None);
    }

    #[test]
    fn latest_release_json_extracts_tag_page_and_exe_asset() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v0.2.0",
            "assets": [
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://example.invalid/SHA256SUMS.txt"},
                {"name": "bdo-optimizer-launcher.exe", "browser_download_url": "https://example.invalid/app.exe"}
            ]
        }"#;

        let release = parse_latest_release_json(json).unwrap();

        assert_eq!(release.tag_name, "v0.2.0");
        assert_eq!(
            release.html_url,
            "https://github.com/owner/repo/releases/tag/v0.2.0"
        );
        assert_eq!(
            release.exe_asset_url.as_deref(),
            Some("https://example.invalid/app.exe")
        );
    }
}
