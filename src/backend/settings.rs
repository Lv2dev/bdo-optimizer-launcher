use crate::backend::schedule;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

// 설정 파일 쓰기 직렬화 락. load→수정→save 구간을 호출처가 이 락 아래에서 수행하면
// 동시 호출(예: apply_mode의 last_user_mode 저장 vs set_setting) 시 갱신 유실을 막는다.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

// 설정 read-modify-write 임계구역 가드. 호출처가 보유하는 동안 다른 writer는 대기한다.
pub fn write_lock() -> MutexGuard<'static, ()> {
    WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

fn default_true() -> bool {
    true
}

pub const UPDATE_CHECK_INTERVAL_6H_MS: u32 = 21_600_000;
pub const UPDATE_CHECK_INTERVAL_12H_MS: u32 = 43_200_000;
pub const UPDATE_CHECK_INTERVAL_1D_MS: u32 = 86_400_000;
pub const UPDATE_CHECK_INTERVAL_3D_MS: u32 = 259_200_000;
pub const UPDATE_CHECK_INTERVAL_7D_MS: u32 = 604_800_000;

pub const UPDATE_CHECK_INTERVAL_OPTIONS_MS: [u32; 5] = [
    UPDATE_CHECK_INTERVAL_6H_MS,
    UPDATE_CHECK_INTERVAL_12H_MS,
    UPDATE_CHECK_INTERVAL_1D_MS,
    UPDATE_CHECK_INTERVAL_3D_MS,
    UPDATE_CHECK_INTERVAL_7D_MS,
];

pub const ACCENT_PALETTE_COUNT: u32 = 4;
pub const MONITOR_INTERVAL_OPTIONS_MS: [u32; 3] = [500, 1000, 2000];

#[derive(thiserror::Error, Debug)]
pub enum SaveError {
    #[error("설정 저장 경로를 확인할 수 없습니다.")]
    PathUnavailable,
    #[error("설정 디렉터리 생성 실패: {0}")]
    CreateDir(std::io::Error),
    #[error("설정 직렬화 실패: {0}")]
    Serialize(serde_json::Error),
    #[error("설정 파일 저장 실패: {0}")]
    Write(std::io::Error),
}

pub fn is_supported_accent_palette(palette: u32) -> bool {
    palette < ACCENT_PALETTE_COUNT
}

pub fn normalize_monitor_interval_ms(ms: u32) -> u32 {
    if MONITOR_INTERVAL_OPTIONS_MS.contains(&ms) {
        ms
    } else {
        default_monitor_interval()
    }
}

// M96 P3: 모니터 폴링 기본 간격(ms). 기존 고정값 1초와 동일하게 둔다.
fn default_monitor_interval() -> u32 {
    1000
}

fn default_update_check_interval() -> u32 {
    UPDATE_CHECK_INTERVAL_1D_MS
}

pub fn is_supported_update_check_interval_ms(ms: u32) -> bool {
    UPDATE_CHECK_INTERVAL_OPTIONS_MS.contains(&ms)
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub theme_mode: ThemeMode,
    #[serde(default)]
    pub accent_palette: u32,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub launcher_path: String,
    // 게임 창이 트레이로 내려가면(IsWindowVisible == FALSE) 자동으로 저전력 모드 적용. 기본 OFF.
    #[serde(default)]
    pub auto_tray_on_game_minimize: bool,
    // X 버튼 클릭 시 트레이로 숨김(기본 ON, 기존 동작 유지). OFF면 완전 종료.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    // M50: 사용자가 명시 선택한 마지막 모드. 게임 신규 실행 감지 시 자동 적용.
    // 자동 저전력(M34) / 자동 규칙(M7) 진입은 persist 안 함.
    #[serde(default)]
    pub last_user_mode: Option<schedule::OptimizeMode>,
    // M96 P3: 게임(BlackDesert64) 신규 실행 감지 시 자동 적용할 기본 모드.
    // None이면 자동 적용하지 않는다(수동). last_user_mode와 별개의 명시적 설정.
    #[serde(default)]
    pub default_mode: Option<schedule::OptimizeMode>,
    // M96 P3: 모니터 탭 자원 폴링 간격(ms). 허용값 500/1000/2000, 기본 1000.
    #[serde(default = "default_monitor_interval")]
    pub monitor_interval_ms: u32,
    // M104: 자동 업데이트 알림. 기본 ON, 주기는 기본 하루.
    #[serde(default = "default_true")]
    pub update_alert_enabled: bool,
    #[serde(default = "default_update_check_interval")]
    pub update_check_interval_ms: u32,
    // 같은 릴리스 버전을 반복 알림하지 않기 위한 마지막 알림 버전.
    #[serde(default)]
    pub last_update_notified_version: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            accent_palette: 0,
            reduce_motion: false,
            launcher_path: String::new(),
            auto_tray_on_game_minimize: false,
            close_to_tray: true,
            last_user_mode: None,
            default_mode: None,
            monitor_interval_ms: 1000,
            update_alert_enabled: true,
            update_check_interval_ms: UPDATE_CHECK_INTERVAL_1D_MS,
            last_update_notified_version: None,
        }
    }
}

// APPDATA가 없으면 None을 반환한다. 설치 폴더(.)에 settings.json을 만들면
// 다중 사용자 환경에서 권한 충돌이 발생하므로 안전한 휘발성 동작을 선택한다.
fn settings_path() -> Option<PathBuf> {
    let base = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(base)
            .join("bdo-optimizer-launcher")
            .join("settings.json"),
    )
}

pub fn load_settings() -> AppSettings {
    let path = match settings_path() {
        Some(p) => p,
        None => return AppSettings::default(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return AppSettings::default(), // 파일 없음/읽기 실패 = 신규 사용자
    };
    match serde_json::from_str::<AppSettings>(&content) {
        Ok(mut settings) => {
            settings.monitor_interval_ms =
                normalize_monitor_interval_ms(settings.monitor_interval_ms);
            settings
        }
        Err(error) => {
            // 손상 JSON을 조용히 기본값으로 덮어쓰지 않고, 원본을 백업하고 경고를 남긴다.
            tracing::warn!(error = %error, "settings.json 파싱 실패 — 손상 파일 백업 후 기본값 사용");
            backup_broken(&path);
            AppSettings::default()
        }
    }
}

fn save_settings_to_path(settings: &AppSettings, path: &std::path::Path) -> Result<(), SaveError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(SaveError::CreateDir)?;
    }
    let json = serde_json::to_vec(settings).map_err(SaveError::Serialize)?;
    super::atomic_write(path, &json).map_err(SaveError::Write)
}

pub fn save_settings(settings: &AppSettings) -> Result<(), SaveError> {
    let path = settings_path().ok_or(SaveError::PathUnavailable)?;
    save_settings_to_path(settings, &path)
}

// schedule.rs와 동일 정책: 깨진 JSON은 빈/기본값으로 덮어쓰지 않도록 타임스탬프 백업.
fn backup_broken(path: &std::path::Path) {
    use chrono::Local;
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let backup = path.with_extension(format!("json.broken-{stamp}"));
    let _ = std::fs::rename(path, backup);
}

pub fn resolve_dark_mode(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
        ThemeMode::System => detect_os_dark_mode(),
    }
}

#[cfg(windows)]
fn detect_os_dark_mode() -> bool {
    let out = super::system_command("reg.exe")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let is_light = text
                .lines()
                .any(|l| l.split_whitespace().last() == Some("0x1"));
            !is_light
        }
        _ => true,
    }
}

#[cfg(not(windows))]
fn detect_os_dark_mode() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_settings_defaults_missing_launcher_path_to_empty() {
        let settings: AppSettings = serde_json::from_str(r#"{"theme_mode":"dark"}"#).unwrap();

        assert_eq!("", settings.launcher_path);
    }

    #[test]
    fn app_settings_round_trips_launcher_path() {
        let settings = AppSettings {
            launcher_path: r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe".to_string(),
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(settings.launcher_path, restored.launcher_path);
    }

    #[test]
    fn app_settings_round_trips_default_mode_and_monitor_interval() {
        let settings = AppSettings {
            default_mode: Some(schedule::OptimizeMode::High),
            monitor_interval_ms: 2000,
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(Some(schedule::OptimizeMode::High), restored.default_mode);
        assert_eq!(2000, restored.monitor_interval_ms);
    }

    #[test]
    fn app_settings_defaults_monitor_interval_and_no_default_mode() {
        let settings: AppSettings = serde_json::from_str(r#"{"theme_mode":"dark"}"#).unwrap();

        assert_eq!(None, settings.default_mode);
        assert_eq!(1000, settings.monitor_interval_ms);
    }

    #[test]
    fn app_settings_defaults_update_alert_to_daily_polling() {
        let settings: AppSettings = serde_json::from_str(r#"{"theme_mode":"dark"}"#).unwrap();

        assert!(settings.update_alert_enabled);
        assert_eq!(86_400_000, settings.update_check_interval_ms);
        assert_eq!(None, settings.last_update_notified_version);
    }

    #[test]
    fn app_settings_round_trips_accent_palette() {
        let settings = AppSettings {
            accent_palette: 2,
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(2, restored.accent_palette);
    }

    #[test]
    fn app_settings_defaults_accent_palette_to_zero() {
        let settings: AppSettings = serde_json::from_str(r#"{"theme_mode":"dark"}"#).unwrap();

        assert_eq!(0, settings.accent_palette);
    }

    #[test]
    fn app_settings_round_trips_update_alert_fields() {
        let settings = AppSettings {
            update_alert_enabled: false,
            update_check_interval_ms: 21_600_000,
            last_update_notified_version: Some("0.1.2".to_string()),
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert!(!restored.update_alert_enabled);
        assert_eq!(21_600_000, restored.update_check_interval_ms);
        assert_eq!(
            Some("0.1.2"),
            restored.last_update_notified_version.as_deref()
        );
    }

    #[test]
    fn save_settings_reports_directory_creation_failure() {
        let blocker = std::env::temp_dir().join(format!(
            "bdo-optimizer-settings-blocker-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&blocker);
        std::fs::write(&blocker, b"file blocks directory").unwrap();

        let result = save_settings_to_path(&AppSettings::default(), &blocker.join("settings.json"));

        std::fs::remove_file(&blocker).unwrap();
        assert!(matches!(result, Err(SaveError::CreateDir(_))));
    }

    #[test]
    fn monitor_interval_normalization_allows_only_ui_contract_values() {
        assert_eq!(500, normalize_monitor_interval_ms(500));
        assert_eq!(1000, normalize_monitor_interval_ms(1000));
        assert_eq!(2000, normalize_monitor_interval_ms(2000));
        assert_eq!(1000, normalize_monitor_interval_ms(1));
        assert_eq!(1000, normalize_monitor_interval_ms(1500));
        assert_eq!(1000, normalize_monitor_interval_ms(u32::MAX));
    }
}
