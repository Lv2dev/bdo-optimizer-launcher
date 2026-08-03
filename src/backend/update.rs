use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::Duration;
use tauri::{ipc::Channel, AppHandle};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_METADATA_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UPDATE_ASSET_PREFIX: &str =
    "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub status_text: String,
    pub latest_version: String,
    pub update_available: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum UpdateProgressEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        content_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    Verifying,
    Installing,
}

#[derive(Default)]
struct PendingUpdateInner {
    pending: Option<Update>,
    check_generation: u64,
}

#[derive(Default)]
pub struct PendingUpdateState {
    inner: Mutex<PendingUpdateInner>,
    installing: AtomicBool,
}

struct InstallClaim<'a>(&'a AtomicBool);

impl Drop for InstallClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl PendingUpdateState {
    fn begin_check(&self) -> Result<u64, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::StatePoisoned)?;
        inner.check_generation = inner
            .check_generation
            .checked_add(1)
            .ok_or(Error::CheckGenerationExhausted)?;
        Ok(inner.check_generation)
    }

    fn replace_if_current(&self, generation: u64, update: Option<Update>) -> Result<bool, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::StatePoisoned)?;
        if inner.check_generation != generation {
            return Ok(false);
        }
        inner.pending = update;
        Ok(true)
    }

    fn take(&self) -> Result<Option<Update>, Error> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| Error::StatePoisoned)?
            .pending
            .take())
    }

    fn restore_if_empty(&self, update: Update) -> Result<(), Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::StatePoisoned)?;
        if inner.pending.is_none() {
            inner.pending = Some(update);
        }
        Ok(())
    }

    fn begin_install(&self) -> Result<InstallClaim<'_>, Error> {
        self.installing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| InstallClaim(&self.installing))
            .map_err(|_| Error::InstallInProgress)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("업데이트 처리 실패: {0}")]
    Updater(#[from] tauri_plugin_updater::Error),
    #[error("업데이트 다운로드 주소가 허용된 GitHub Release 자산이 아닙니다.")]
    InvalidDownloadUrl,
    #[error("설치할 업데이트가 없습니다. 다시 확인해 주세요.")]
    NoPendingUpdate,
    #[error("업데이트 설치가 이미 진행 중입니다.")]
    InstallInProgress,
    #[error("업데이트 상태 잠금이 손상되었습니다.")]
    StatePoisoned,
    #[error("업데이트 확인 요청 번호를 더 이상 만들 수 없습니다.")]
    CheckGenerationExhausted,
    #[error("더 최신 업데이트 확인 요청이 시작되어 이전 결과를 폐기했습니다.")]
    StaleCheck,
    #[error("릴리스 페이지 URL이 올바르지 않습니다.")]
    InvalidReleaseUrl,
    #[error("브라우저 실행 실패: {0}")]
    ExplorerSpawn(#[from] std::io::Error),
}

fn sanitize_notes(notes: Option<String>) -> Option<String> {
    notes.and_then(|notes| {
        let trimmed = notes.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(4_000).collect())
        }
    })
}

fn available_update_check(version: String, notes: Option<String>) -> UpdateCheck {
    UpdateCheck {
        status_text: format!("새 버전 {version} 사용 가능."),
        latest_version: version,
        update_available: true,
        notes: sanitize_notes(notes),
    }
}

fn current_update_check() -> UpdateCheck {
    let current = env!("APP_VERSION").to_string();
    UpdateCheck {
        status_text: format!("최신 버전입니다. ({current})"),
        latest_version: current,
        update_available: false,
        notes: None,
    }
}

fn ensure_current_check(replaced: bool) -> Result<(), Error> {
    replaced.then_some(()).ok_or(Error::StaleCheck)
}

fn is_allowed_update_download_url(url: &str) -> bool {
    let Some(path) = url.strip_prefix(UPDATE_ASSET_PREFIX) else {
        return false;
    };
    if path.contains('@') || path.contains(['?', '#']) {
        return false;
    }
    let mut segments = path.split('/');
    let tag = segments.next().unwrap_or_default();
    let file_name = segments.next().unwrap_or_default();
    !tag.is_empty() && !file_name.is_empty() && segments.next().is_none()
}

pub async fn check_latest_release(
    app: &AppHandle,
    pending: &PendingUpdateState,
) -> Result<UpdateCheck, Error> {
    let generation = pending.begin_check()?;
    let update = app
        .updater_builder()
        .timeout(UPDATE_METADATA_TIMEOUT)
        .build()?
        .check()
        .await?;

    let Some(update) = update else {
        ensure_current_check(pending.replace_if_current(generation, None)?)?;
        return Ok(current_update_check());
    };

    if !is_allowed_update_download_url(update.download_url.as_str()) {
        ensure_current_check(pending.replace_if_current(generation, None)?)?;
        return Err(Error::InvalidDownloadUrl);
    }

    let mut update = update;
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);
    let check = available_update_check(update.version.clone(), update.body.clone());
    ensure_current_check(pending.replace_if_current(generation, Some(update))?)?;
    Ok(check)
}

pub async fn install_pending_update(
    pending: &PendingUpdateState,
    on_event: Channel<UpdateProgressEvent>,
) -> Result<(), Error> {
    let _claim = pending.begin_install()?;
    let update = pending.take()?.ok_or(Error::NoPendingUpdate)?;
    let retry_update = update.clone();
    let progress_channel = on_event.clone();
    let verifying_channel = on_event.clone();
    let mut downloaded = 0_u64;
    let mut started = false;

    let result = async {
        let bytes = update
            .download(
                move |chunk_length, content_length| {
                    if !started {
                        started = true;
                        let _ =
                            progress_channel.send(UpdateProgressEvent::Started { content_length });
                    }
                    downloaded = downloaded.saturating_add(chunk_length as u64);
                    let _ = progress_channel.send(UpdateProgressEvent::Progress {
                        downloaded,
                        content_length,
                    });
                },
                move || {
                    let _ = verifying_channel.send(UpdateProgressEvent::Verifying);
                },
            )
            .await?;

        let _ = on_event.send(UpdateProgressEvent::Installing);
        update.install(bytes)
    }
    .await;

    if let Err(error) = result {
        pending.restore_if_empty(retry_update)?;
        return Err(error.into());
    }
    Ok(())
}

// 외부 URL을 여는 관리자 프로세스 표면은 GitHub 호스트로 제한한다.
fn is_allowed_release_url(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
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
    fn updater_accepts_only_exact_repository_release_assets() {
        assert!(is_allowed_update_download_url(
            "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/v0.3.0/bdo-optimizer-launcher-setup.exe"
        ));
        assert!(!is_allowed_update_download_url(
            "http://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/v0.3.0/setup.exe"
        ));
        assert!(!is_allowed_update_download_url(
            "https://github.com/other/repo/releases/download/v0.3.0/setup.exe"
        ));
        assert!(!is_allowed_update_download_url(
            "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/v0.3.0/setup.exe?redirect=evil"
        ));
        assert!(!is_allowed_update_download_url(
            "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/v0.3.0/sub/setup.exe"
        ));
    }

    #[test]
    fn update_metadata_trims_and_bounds_remote_notes() {
        let check = available_update_check("0.3.0".to_string(), Some("  변경 사항  ".into()));
        assert_eq!(check.notes.as_deref(), Some("변경 사항"));

        let long = "가".repeat(4_001);
        assert_eq!(sanitize_notes(Some(long)).unwrap().chars().count(), 4_000);
        assert_eq!(sanitize_notes(Some("  ".into())), None);
    }

    #[test]
    fn progress_event_uses_stable_camel_case_wire_shape() {
        assert_eq!(
            serde_json::to_value(UpdateProgressEvent::Started {
                content_length: Some(123)
            })
            .unwrap(),
            serde_json::json!({
                "event": "started",
                "data": { "contentLength": 123 }
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateProgressEvent::Progress {
                downloaded: 10,
                content_length: None
            })
            .unwrap(),
            serde_json::json!({
                "event": "progress",
                "data": { "downloaded": 10, "contentLength": null }
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateProgressEvent::Verifying).unwrap(),
            serde_json::json!({ "event": "verifying" })
        );
        assert_eq!(
            serde_json::to_value(UpdateProgressEvent::Installing).unwrap(),
            serde_json::json!({ "event": "installing" })
        );
    }

    #[test]
    fn install_claim_is_single_flight_and_releases_on_drop() {
        let state = PendingUpdateState::default();
        let claim = state.begin_install().unwrap();
        assert!(matches!(
            state.begin_install(),
            Err(Error::InstallInProgress)
        ));
        drop(claim);
        assert!(state.begin_install().is_ok());
    }

    #[test]
    fn stale_update_check_is_rejected_before_callers_apply_side_effects() {
        let state = PendingUpdateState::default();
        let older = state.begin_check().unwrap();
        let newer = state.begin_check().unwrap();

        let stale_replacement = state.replace_if_current(older, None).unwrap();
        assert!(matches!(
            ensure_current_check(stale_replacement),
            Err(Error::StaleCheck)
        ));
        assert!(state.replace_if_current(newer, None).unwrap());
    }

    #[test]
    fn metadata_and_artifact_download_use_separate_timeouts() {
        assert_eq!(UPDATE_METADATA_TIMEOUT, Duration::from_secs(15));
        assert_eq!(UPDATE_DOWNLOAD_TIMEOUT, Duration::from_secs(10 * 60));
        assert!(UPDATE_DOWNLOAD_TIMEOUT > UPDATE_METADATA_TIMEOUT);
    }

    #[test]
    fn release_url_allows_only_github_hosts() {
        assert!(is_allowed_release_url(
            "https://github.com/owner/repo/releases/tag/v0.2.0"
        ));
        assert!(is_allowed_release_url(
            "https://api.github.com/repos/o/r/releases/latest"
        ));
        assert!(!is_allowed_release_url("http://github.com/owner/repo"));
        assert!(!is_allowed_release_url("https://github.com.evil.com/x"));
        assert!(!is_allowed_release_url("https://github.com@evil.com/x"));
        assert!(!is_allowed_release_url("file:///c:/windows/system32"));
    }

    #[test]
    #[ignore = "release updater artifact 경로가 필요합니다"]
    fn release_artifact_signature_matches_embedded_public_key() {
        use base64::Engine as _;
        use minisign_verify::{PublicKey, Signature};

        let artifact_path = std::env::var("UPDATER_ARTIFACT")
            .expect("UPDATER_ARTIFACT must point to the release installer");
        let signature_path = std::env::var("UPDATER_SIGNATURE")
            .expect("UPDATER_SIGNATURE must point to the installer signature");
        let config_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
        let encoded_public_key = config["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("updater public key is missing");
        let public_key_text = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded_public_key)
                .expect("updater public key is not base64"),
        )
        .expect("updater public key is not UTF-8");
        let public_key = PublicKey::decode(&public_key_text).expect("invalid updater public key");

        let encoded_signature = std::fs::read_to_string(signature_path).unwrap();
        let signature_text = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded_signature.trim())
                .expect("updater signature is not base64"),
        )
        .expect("updater signature is not UTF-8");
        let signature = Signature::decode(&signature_text).expect("invalid updater signature");
        let artifact = std::fs::read(artifact_path).unwrap();

        public_key
            .verify(&artifact, &signature, true)
            .expect("artifact signature does not match the embedded public key");

        let mut tampered = artifact;
        let first = tampered.first_mut().expect("updater artifact is empty");
        *first ^= 1;
        assert!(public_key.verify(&tampered, &signature, true).is_err());
    }
}
