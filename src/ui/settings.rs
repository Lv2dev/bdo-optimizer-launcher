use crate::backend::{autostart, settings};
use crate::ui::push_status;
use crate::ThemeMode as UiThemeMode;
use crate::{AppWindow, Theme};
use slint::ComponentHandle;

fn to_ui_mode(m: settings::ThemeMode) -> UiThemeMode {
    match m {
        settings::ThemeMode::Light => UiThemeMode::Light,
        settings::ThemeMode::Dark => UiThemeMode::Dark,
        settings::ThemeMode::System => UiThemeMode::System,
    }
}

fn to_backend_mode(m: UiThemeMode) -> settings::ThemeMode {
    match m {
        UiThemeMode::Light => settings::ThemeMode::Light,
        UiThemeMode::Dark => settings::ThemeMode::Dark,
        UiThemeMode::System => settings::ThemeMode::System,
    }
}

pub fn apply_initial(app: &AppWindow) -> settings::AppSettings {
    let initial_settings = settings::load_settings();
    let dark = settings::resolve_dark_mode(initial_settings.theme_mode);
    app.global::<Theme>().set_dark(dark);
    app.set_reduce_motion(initial_settings.reduce_motion);
    app.global::<Theme>()
        .set_motion_scale(if initial_settings.reduce_motion {
            0.0
        } else {
            1.0
        });
    app.set_theme_mode(to_ui_mode(initial_settings.theme_mode));
    app.set_launcher_path(initial_settings.launcher_path.clone().into());
    app.set_auto_tray_on_game_minimize(initial_settings.auto_tray_on_game_minimize);
    app.set_close_to_tray(initial_settings.close_to_tray);
    // 자동 시작은 schtasks가 진실의 근거. 매 진입 시 query로 동기화.
    let (enabled, minimized) = autostart::query_autostart();
    app.set_autostart_enabled(enabled);
    app.set_autostart_minimized(minimized);
    app.set_active_tab(0);
    app.set_shutdown_expanded(true);
    app.set_auto_mode_expanded(false);
    app.set_prev_status_text("".into());
    initial_settings
}

pub fn register(app: &AppWindow) {
    app.on_change_theme({
        let app = app.as_weak();
        move |mode| {
            if let Some(app) = app.upgrade() {
                let theme_mode = to_backend_mode(mode);
                let dark = settings::resolve_dark_mode(theme_mode);
                app.global::<Theme>().set_dark(dark);
                app.set_theme_mode(mode);
                let mut s = settings::load_settings();
                s.theme_mode = theme_mode;
                settings::save_settings(&s);
            }
        }
    });

    app.on_toggle_reduce_motion({
        let app = app.as_weak();
        move |on| {
            if let Some(app) = app.upgrade() {
                app.set_reduce_motion(on);
                app.global::<Theme>()
                    .set_motion_scale(if on { 0.0 } else { 1.0 });
                let mut s = settings::load_settings();
                s.reduce_motion = on;
                settings::save_settings(&s);
            }
        }
    });

    app.on_toggle_auto_tray({
        let app = app.as_weak();
        move |on| {
            if let Some(app) = app.upgrade() {
                app.set_auto_tray_on_game_minimize(on);
                let mut s = settings::load_settings();
                s.auto_tray_on_game_minimize = on;
                settings::save_settings(&s);
            }
        }
    });

    app.on_toggle_close_to_tray({
        let app = app.as_weak();
        move |on| {
            if let Some(app) = app.upgrade() {
                app.set_close_to_tray(on);
                let mut s = settings::load_settings();
                s.close_to_tray = on;
                settings::save_settings(&s);
            }
        }
    });

    app.on_launcher_path_changed({
        let app = app.as_weak();
        move |path| {
            if let Some(app) = app.upgrade() {
                app.set_launcher_path(path.clone());
                let mut s = settings::load_settings();
                s.launcher_path = path.to_string();
                settings::save_settings(&s);
            }
        }
    });

    // 자동 시작 토글: schtasks 등록/해제. 실패 시 UI 상태 원복.
    app.on_toggle_autostart({
        let app = app.as_weak();
        move |on| {
            if let Some(app) = app.upgrade() {
                let result = if on {
                    autostart::register_autostart(app.get_autostart_minimized())
                } else {
                    autostart::unregister_autostart()
                };
                match result {
                    Ok(()) => {
                        app.set_autostart_enabled(on);
                        push_status(
                            &app,
                            if on {
                                "자동 시작을 등록했습니다."
                            } else {
                                "자동 시작을 해제했습니다."
                            },
                        );
                    }
                    Err(e) => {
                        // 실패 시 query로 실제 상태를 다시 읽어 동기화.
                        let (real_on, real_min) = autostart::query_autostart();
                        app.set_autostart_enabled(real_on);
                        app.set_autostart_minimized(real_min);
                        push_status(&app, e.to_string());
                    }
                }
            }
        }
    });

    // 트레이 시작 토글: 등록된 상태에서만 효과. /create /f가 overwrite하므로 재등록.
    app.on_toggle_autostart_minimized({
        let app = app.as_weak();
        move |on| {
            if let Some(app) = app.upgrade() {
                if !app.get_autostart_enabled() {
                    return;
                }
                match autostart::register_autostart(on) {
                    Ok(()) => {
                        app.set_autostart_minimized(on);
                        push_status(
                            &app,
                            if on {
                                "자동 시작을 트레이 시작으로 변경했습니다."
                            } else {
                                "자동 시작을 일반 창으로 변경했습니다."
                            },
                        );
                    }
                    Err(e) => {
                        let (real_on, real_min) = autostart::query_autostart();
                        app.set_autostart_enabled(real_on);
                        app.set_autostart_minimized(real_min);
                        push_status(&app, e.to_string());
                    }
                }
            }
        }
    });

    // M64: 진단 카드 — 로그 폴더 열기 (explorer.exe %LOCALAPPDATA%\bdo-optimizer-launcher\logs\).
    app.on_open_log_folder({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                match crate::backend::logging::open_log_folder() {
                    Ok(()) => {
                        tracing::info!("log folder opened");
                        push_status(&app, "로그 폴더를 열었습니다.");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "open log folder failed");
                        push_status(&app, e.to_string());
                    }
                }
            }
        }
    });
}

pub fn start_ripple_timer(app: &AppWindow) -> slint::Timer {
    // M89-2: ripple decay를 더 부드럽게 — 50ms tick × 0.05 감쇠로 약 1000ms 지속.
    // 기존 33ms × 0.06 ≈ 550ms 대비 약 1.8배 길어 모드 적용 피드백이 더 명확하게 사라짐.
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        {
            let app = app.as_weak();
            move || {
                if let Some(app) = app.upgrade() {
                    let v = app.get_mode_ripple();
                    if v <= 0.0 {
                        return;
                    }
                    let next = (v - 0.05).max(0.0);
                    app.set_mode_ripple(next);
                }
            }
        },
    );
    timer
}
