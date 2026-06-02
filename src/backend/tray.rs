// Tauri tray/window lifecycle에서 재사용하는 표시 문자열 helper.
// raw tray-icon 기반 생성/이벤트 루프는 M94-8에서 제거되었다.

pub fn mode_menu_labels(
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

pub fn mode_tooltip(mode: Option<crate::backend::schedule::OptimizeMode>) -> String {
    let label = match mode {
        Some(crate::backend::schedule::OptimizeMode::High) => "고성능 모드",
        Some(crate::backend::schedule::OptimizeMode::Normal) => "일반 모드",
        Some(crate::backend::schedule::OptimizeMode::LowPower) => "저전력 모드",
        None => "알 수 없음",
    };
    format!("검은사막 최적화 런처 | 현재 모드: {label}")
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
