use crate::backend::schedule;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
        let _ = std::fs::write(path, json);
    }
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
