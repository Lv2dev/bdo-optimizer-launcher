#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
slint::include_modules!();
mod backend;
mod ui;

use ui::AppState;

fn main() -> Result<(), slint::PlatformError> {
    // M63: 파일 로거 초기화. 단일 인스턴스 가드 전에 호출해 시작/종료 흐름을 모두 기록.
    // guard는 main scope 끝까지 살아야 비동기 writer가 flush.
    let _log_guard = backend::logging::init();
    tracing::info!(args = ?std::env::args().collect::<Vec<_>>(), "main start");

    // 단일 인스턴스 보장: 두 번째 실행이면 기존 창 포그라운드/복원 후 즉시 종료.
    if !backend::singleton::acquire_or_focus_existing() {
        tracing::info!("singleton: existing instance found, exiting");
        std::process::exit(0);
    }

    let app = AppWindow::new()?;

    // M62-1: build.rs에서 주입된 Cargo.toml 버전을 Slint property로 전달.
    app.set_app_version(env!("APP_VERSION").into());

    // 초기 상태 적용 (테마 등 전역 영향 → settings 먼저).
    let _initial_settings = ui::settings::apply_initial(&app);

    // AppState 공유 상태 생성.
    let state = AppState::new();

    // 트레이 init: 성공/실패에 따라 이벤트 루프 선택. close-requested까지 일괄 처리.
    let (tray_handle, _tray_timer) = ui::tray_ui::register(&app, &state);

    // 초기 UI 상태.
    ui::calendar::apply_initial(&app);
    ui::shutdown::apply_initial(&app);
    ui::schedule_ui::apply_initial(&app, &state);
    ui::control::apply_initial(&app, &state);
    ui::monitor_ui::apply_initial(&app);
    ui::updates::apply_initial(&app);

    // 콜백 등록.
    ui::settings::register(&app);
    ui::calendar::register(&app);
    ui::shutdown::register(&app);
    ui::schedule_ui::register(&app, &state);
    ui::control::register(&app, &state);
    ui::monitor_ui::register(&app);
    ui::updates::register(&app);

    // Timer 시작 (binding을 main 라이프타임에 유지).
    let _auto_mode_timer = ui::schedule_ui::start_auto_mode_timer(&app, &state);
    let _status_timer = ui::control::start_status_timer(&app, &state);
    let _monitor_timer = ui::monitor_ui::start_monitor_timer(&app, &state);
    let _ripple_timer = ui::settings::start_ripple_timer(&app);

    // --minimized: 자동 시작 시 트레이로만 상주. 트레이 init 실패 시 무시(안전망).
    let start_minimized = std::env::args().any(|a| a == "--minimized");

    // 트레이 상주 모드 분기.
    if let Some(ref handle) = tray_handle {
        if !start_minimized {
            app.show()?;
        } else {
            // 창을 숨긴 채로 시작하므로 트레이 토글 라벨을 "창 열기"로 동기화.
            backend::tray::set_toggle_label(handle, false);
        }
        slint::run_event_loop_until_quit()
    } else {
        app.run()
    }
}
