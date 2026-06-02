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

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub theme_mode: ThemeMode,
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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            reduce_motion: false,
            launcher_path: String::new(),
            auto_tray_on_game_minimize: false,
            close_to_tray: true,
            last_user_mode: None,
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
    match serde_json::from_str(&content) {
        Ok(settings) => settings,
        Err(error) => {
            // 손상 JSON을 조용히 기본값으로 덮어쓰지 않고, 원본을 백업하고 경고를 남긴다.
            tracing::warn!(error = %error, "settings.json 파싱 실패 — 손상 파일 백업 후 기본값 사용");
            backup_broken(&path);
            AppSettings::default()
        }
    }
}

pub fn save_settings(settings: &AppSettings) {
    let path = match settings_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(settings) {
        let _ = super::atomic_write(&path, json.as_bytes());
    }
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
}
