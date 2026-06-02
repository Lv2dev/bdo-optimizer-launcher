// 시스템 트레이 아이콘 + 우클릭 메뉴.
// tray-icon crate는 Windows에서 hidden window를 만들어 자체 메시지 펌프를 돌리므로
// 본 모듈은 main 스레드에서 1회 초기화한 뒤 TrayIcon 핸들을 그대로 유지한다.
// 메뉴/아이콘 이벤트는 main.rs의 워커 스레드가 polling하여 invoke_from_event_loop로 디스패치한다.

use tray_icon::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

pub struct TrayHandle {
    // 트레이 아이콘은 Drop 시 시스템 트레이에서 제거되므로 핸들을 살려두기 위해 보유한다.
    #[allow(dead_code)]
    pub icon: TrayIcon,
    pub menu_id_toggle: String,
    pub menu_id_high: String,
    pub menu_id_normal: String,
    pub menu_id_low_power: String,
    pub menu_id_cancel_shutdown: String,
    pub menu_id_quit: String,
    pub toggle_item: MenuItem,
    pub high_item: MenuItem,
    pub normal_item: MenuItem,
    pub low_power_item: MenuItem,
}

// assets/tray_16.png을 컴파일 타임에 임베드해 트레이 아이콘으로 사용한다.
// 16x16 RGBA PNG 트레이 아이콘.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../assets/tray_16.png");

// M66b: thiserror enum.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("트레이 아이콘 디코드 실패: {0}")]
    IconDecode(String),
    #[error("트레이 아이콘 생성 실패: {0}")]
    IconCreate(String),
    #[error("메뉴 추가 실패({slot}): {detail}")]
    MenuAppend { slot: &'static str, detail: String },
    #[error("트레이 아이콘 빌드 실패: {0}")]
    TrayBuild(String),
}

fn build_default_icon() -> Result<Icon, Error> {
    let img = image::load_from_memory(TRAY_ICON_PNG)
        .map_err(|e| Error::IconDecode(e.to_string()))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    Icon::from_rgba(img.into_raw(), w, h).map_err(|e| Error::IconCreate(e.to_string()))
}

// 트레이 아이콘과 메뉴를 빌드한다. main 스레드에서 1회 호출한다.
pub fn build() -> Result<TrayHandle, Error> {
    let icon = build_default_icon()?;

    let menu = Menu::new();

    let toggle_item = MenuItem::new("창 숨기기", true, None);
    let high_item = MenuItem::new("고성능 모드 적용", true, None);
    let normal_item = MenuItem::new("일반 모드 적용", true, None);
    let low_power_item = MenuItem::new("저전력 모드 적용", true, None);
    let cancel_item = MenuItem::new("예약 종료 취소", true, None);
    let sep = PredefinedMenuItem::separator();
    let quit_item = MenuItem::new("종료", true, None);

    let menu_id_toggle = toggle_item.id().0.clone();
    let menu_id_high = high_item.id().0.clone();
    let menu_id_normal = normal_item.id().0.clone();
    let menu_id_low_power = low_power_item.id().0.clone();
    let menu_id_cancel_shutdown = cancel_item.id().0.clone();
    let menu_id_quit = quit_item.id().0.clone();

    let append =
        |slot: &'static str, item: &dyn tray_icon::menu::IsMenuItem| -> Result<(), Error> {
            menu.append(item).map_err(|e| Error::MenuAppend {
                slot,
                detail: e.to_string(),
            })
        };
    append("toggle", &toggle_item)?;
    append("high", &high_item)?;
    append("normal", &normal_item)?;
    append("low_power", &low_power_item)?;
    append("cancel", &cancel_item)?;
    append("sep", &sep)?;
    append("quit", &quit_item)?;

    let icon_handle = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("검은사막 최적화 런처")
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| Error::TrayBuild(e.to_string()))?;

    Ok(TrayHandle {
        icon: icon_handle,
        menu_id_toggle,
        menu_id_high,
        menu_id_normal,
        menu_id_low_power,
        menu_id_cancel_shutdown,
        menu_id_quit,
        toggle_item,
        high_item,
        normal_item,
        low_power_item,
    })
}

// MenuItem이 내부 interior mutability(Arc<Mutex>)를 보유하므로 &self로 라벨 변경이 가능하다.
pub fn set_toggle_label(handle: &TrayHandle, window_visible: bool) {
    let label = if window_visible {
        "창 숨기기"
    } else {
        "창 열기"
    };
    handle.toggle_item.set_text(label);
}

fn mode_menu_labels(
    mode: Option<crate::backend::schedule::OptimizeMode>,
) -> (&'static str, &'static str, &'static str) {
    use crate::backend::schedule::OptimizeMode;

    match mode {
        Some(OptimizeMode::High) => (
            "[현재] 고성능 모드 적용",
            "일반 모드 적용",
            "저전력 모드 적용",
        ),
        Some(OptimizeMode::Normal) => (
            "고성능 모드 적용",
            "[현재] 일반 모드 적용",
            "저전력 모드 적용",
        ),
        Some(OptimizeMode::LowPower) => (
            "고성능 모드 적용",
            "일반 모드 적용",
            "[현재] 저전력 모드 적용",
        ),
        None => ("고성능 모드 적용", "일반 모드 적용", "저전력 모드 적용"),
    }
}

fn mode_tooltip(mode: Option<crate::backend::schedule::OptimizeMode>) -> String {
    let label = match mode {
        Some(crate::backend::schedule::OptimizeMode::High) => "고성능 모드",
        Some(crate::backend::schedule::OptimizeMode::Normal) => "일반 모드",
        Some(crate::backend::schedule::OptimizeMode::LowPower) => "저전력 모드",
        None => "알 수 없음",
    };
    format!("검은사막 최적화 런처 | 현재 모드: {label}")
}

pub fn set_mode_indicator(
    handle: &TrayHandle,
    mode: Option<crate::backend::schedule::OptimizeMode>,
) {
    let (high, normal, low_power) = mode_menu_labels(mode);
    handle.high_item.set_text(high);
    handle.normal_item.set_text(normal);
    handle.low_power_item.set_text(low_power);
    let _ = handle.icon.set_tooltip(Some(mode_tooltip(mode)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::schedule::OptimizeMode;

    #[test]
    fn mode_menu_labels_marks_high_only() {
        assert_eq!(
            mode_menu_labels(Some(OptimizeMode::High)),
            (
                "[현재] 고성능 모드 적용",
                "일반 모드 적용",
                "저전력 모드 적용",
            )
        );
    }

    #[test]
    fn mode_menu_labels_resets_when_unknown() {
        assert_eq!(
            mode_menu_labels(None),
            ("고성능 모드 적용", "일반 모드 적용", "저전력 모드 적용",)
        );
    }

    #[test]
    fn mode_tooltip_includes_unknown_label() {
        assert_eq!(
            mode_tooltip(None),
            "검은사막 최적화 런처 | 현재 모드: 알 수 없음"
        );
    }
}
