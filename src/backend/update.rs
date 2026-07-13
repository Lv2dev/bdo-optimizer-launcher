use serde::Deserialize;
use std::time::Duration;

const DEFAULT_GITHUB_REPOSITORY: &str = "Lv2dev/bdo-optimizer-launcher";

#[derive(Clone, Copy)]
struct UpdateHttpTimeouts {
    connect: Duration,
    read: Duration,
    total: Duration,
}

fn update_http_timeouts() -> UpdateHttpTimeouts {
    UpdateHttpTimeouts {
        connect: Duration::from_secs(5),
        read: Duration::from_secs(10),
        total: Duration::from_secs(15),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestRelease {
    pub tag_name: String,
    pub html_url: String,
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
    #[error("업데이트 확인 시간이 초과되었습니다.")]
    Timeout,
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
}

pub fn release_api_url(
    explicit_url: Option<&str>,
    github_repository: Option<&str>,
) -> Option<String> {
    if let Some(url) = explicit_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    github_repository
        .map(str::trim)
        .filter(|repo| !repo.is_empty())
        .map(|repo| format!("https://api.github.com/repos/{repo}/releases/latest"))
}

pub fn configured_release_api_url() -> Option<String> {
    release_api_url(
        option_env!("UPDATE_RELEASES_API_URL"),
        Some(DEFAULT_GITHUB_REPOSITORY),
    )
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
    Ok(LatestRelease {
        tag_name: release.tag_name,
        html_url: release.html_url,
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
    fetch_latest_release_json_with_timeouts(url, update_http_timeouts())
}

fn fetch_latest_release_json_with_timeouts(
    url: &str,
    timeouts: UpdateHttpTimeouts,
) -> Result<String, Error> {
    let agent = ureq::builder()
        .timeout_connect(timeouts.connect)
        .timeout_read(timeouts.read)
        .timeout(timeouts.total)
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", "bdo-optimizer-launcher")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(map_ureq_error)?;
    response.into_string().map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            Error::Timeout
        } else {
            Error::Http(error.to_string())
        }
    })
}

fn map_ureq_error(error: ureq::Error) -> Error {
    if let ureq::Error::Transport(transport) = &error {
        let mut source = std::error::Error::source(transport);
        while let Some(current) = source {
            if current
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut)
            {
                return Error::Timeout;
            }
            source = current.source();
        }
    }
    Error::Http(error.to_string())
}

pub fn check_latest_release() -> Result<UpdateCheck, Error> {
    let url = configured_release_api_url().ok_or(Error::ChannelNotConfigured)?;
    let json = fetch_latest_release_json(&url)?;
    let release = parse_latest_release_json(&json)?;
    evaluate_release(env!("APP_VERSION"), release)
}

// release URL은 GitHub Release 페이지(html_url)만 허용한다. requireAdministrator
// 프로세스가 explorer.exe로 임의 https URL을 여는 표면을 막기 위해 호스트를
// github.com 계열로 화이트리스트하고, userinfo(@)·하위도메인/포트 우회 트릭을 거부한다.
fn is_allowed_release_url(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // userinfo(`github.com@evil.com`)는 호스트 위장에 쓰이므로 authority에 '@'가 있으면 거부.
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "github.com" || host.ends_with(".github.com")
}

pub fn open_release_page(url: &str) -> Result<(), Error> {
    let trimmed = url.trim();
    if !is_allowed_release_url(trimmed) {
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
        assert_eq!(
            release_api_url(Some("   "), Some("owner/repo")),
            Some("https://api.github.com/repos/owner/repo/releases/latest".to_string())
        );
    }

    #[test]
    fn configured_release_api_url_defaults_to_public_repository() {
        assert_eq!(
            configured_release_api_url(),
            Some(
                "https://api.github.com/repos/Lv2dev/bdo-optimizer-launcher/releases/latest"
                    .to_string()
            )
        );
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
    fn latest_release_json_extracts_tag_and_release_page() {
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
    }

    #[test]
    fn release_url_allows_only_github_hosts() {
        assert!(is_allowed_release_url(
            "https://github.com/owner/repo/releases/tag/v0.2.0"
        ));
        assert!(is_allowed_release_url(
            "https://api.github.com/repos/o/r/releases/latest"
        ));
        assert!(is_allowed_release_url("  https://github.com/owner/repo  "));
        assert!(is_allowed_release_url("https://github.com"));
    }

    #[test]
    fn release_url_rejects_non_github_and_spoofing_tricks() {
        assert!(!is_allowed_release_url("http://github.com/owner/repo")); // https 아님
        assert!(!is_allowed_release_url("https://evil.com/x"));
        assert!(!is_allowed_release_url("https://evilgithub.com/x")); // suffix 트릭
        assert!(!is_allowed_release_url("https://github.com.evil.com/x")); // 하위도메인 위장
        assert!(!is_allowed_release_url("https://github.com@evil.com/x")); // userinfo 위장
        assert!(!is_allowed_release_url("https://github.com:443@evil.com/")); // userinfo+포트
        assert!(!is_allowed_release_url("file:///c:/windows/system32"));
        assert!(!is_allowed_release_url("https://"));
        assert!(!is_allowed_release_url(""));
    }

    #[test]
    fn update_http_timeouts_are_finite_and_ordered() {
        let timeouts = update_http_timeouts();
        assert!(timeouts.connect > std::time::Duration::ZERO);
        assert!(timeouts.read >= timeouts.connect);
        assert!(timeouts.total >= timeouts.read);
    }

    #[test]
    fn stalled_update_response_returns_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(250));
        });
        let result = fetch_latest_release_json_with_timeouts(
            &format!("http://{address}/release"),
            UpdateHttpTimeouts {
                connect: Duration::from_millis(50),
                read: Duration::from_millis(50),
                total: Duration::from_millis(100),
            },
        );
        assert!(matches!(result, Err(Error::Timeout)));
        server.join().unwrap();
    }
}
