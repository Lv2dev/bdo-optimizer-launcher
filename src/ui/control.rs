use crate::backend::{admin, fps, launcher, process, schedule, settings, window};
use crate::ui::{push_status, AppState};
use crate::{AppWindow, OptimizeMode as UiOptimizeMode};
use slint::ComponentHandle;
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::System::Threading::{
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
};

// M79: backend OptimizeMode → Slint generated UiOptimizeMode 변환.
// schedule_ui::ui_mode_to_backend 역방향. set_current_mode에 사용.
fn backend_to_ui_mode(m: schedule::OptimizeMode) -> UiOptimizeMode {
    match m {
        schedule::OptimizeMode::High => UiOptimizeMode::High,
        schedule::OptimizeMode::Normal => UiOptimizeMode::Normal,
        schedule::OptimizeMode::LowPower => UiOptimizeMode::LowPower,
    }
}

// M79/M87: query_current_mode 결과를 AppWindow와 tray indicator에 일관 반영.
// Some(mode) → set_current_mode(mode) + set_current_mode_known(true)
// None       → set_current_mode_known(false) (모드 enum 자체는 직전 값 유지)
fn set_current_mode_state(app: &AppWindow, state: &AppState, mode: Option<schedule::OptimizeMode>) {
    match mode {
        Some(m) => {
            app.set_current_mode(backend_to_ui_mode(m));
            app.set_current_mode_known(true);
        }
        None => {
            app.set_current_mode_known(false);
        }
    }
    let tray_handle = state.mode.tray_handle.borrow().as_ref().map(Rc::clone);
    if let Some(handle) = tray_handle {
        crate::backend::tray::set_mode_indicator(handle.as_ref(), mode);
    }
}

// M78: 모드별 UI 라벨 단일 진실의 근거. query_current_mode/mode_params 모두
// 이 함수를 거쳐 라벨 문구를 결정한다. 라벨 변경 시 한 곳만 수정.
pub fn mode_label(mode: schedule::OptimizeMode) -> &'static str {
    match mode {
        schedule::OptimizeMode::High => "고성능 모드",
        schedule::OptimizeMode::Normal => "일반 모드",
        schedule::OptimizeMode::LowPower => "저전력 모드",
    }
}

// 모드별 affinity / priority / 사용자 메시지 한 곳에서 결정.
pub fn mode_params(
    mode: &schedule::OptimizeMode,
    info: &process::CpuInfo,
) -> (usize, PROCESS_CREATION_FLAGS, &'static str, &'static str) {
    let label = mode_label(*mode);
    match mode {
        schedule::OptimizeMode::High => (
            process::calc_high_affinity(info),
            HIGH_PRIORITY_CLASS,
            "고성능 모드 적용 완료.",
            label,
        ),
        schedule::OptimizeMode::Normal => (
            process::calc_normal_affinity(info),
            NORMAL_PRIORITY_CLASS,
            "일반 모드 적용 완료.",
            label,
        ),
        schedule::OptimizeMode::LowPower => (
            process::calc_low_power_affinity(info),
            IDLE_PRIORITY_CLASS,
            "저전력 모드 적용 완료.",
            label,
        ),
    }
}

// M50: 사용자가 명시 선택한 모드를 settings.json에 즉시 persist.
// 자동 저전력(M34) / 자동 규칙(M7) 경로는 호출하지 않는다.
// settings_path()가 None이거나 디스크 쓰기 실패는 무시(핵심 모드 적용은 정상 동작).
// M78: apply_user_mode 외 호출처 없음 → private 강등. persist 누락 도메인 규칙을
// 함수 이름에 인코딩.
fn persist_last_user_mode(mode: schedule::OptimizeMode) {
    let mut s = settings::load_settings();
    s.last_user_mode = Some(mode);
    settings::save_settings(&s);
}

// M78: 사용자가 명시 선택한 모드 적용 = apply_mode + persist 묶음.
// 수동 버튼(고성능/일반/저전력) + 트레이 메뉴 dispatch 모두 이 함수를 호출.
// 자동 규칙(schedule_ui::start_auto_mode_timer) + 자동 저전력(status_timer)은
// apply_mode만 호출하고 persist는 호출하지 않는다.
pub fn apply_user_mode(app: &AppWindow, state: &AppState, mode: schedule::OptimizeMode) {
    apply_mode(app, state, mode);
    persist_last_user_mode(mode);
}

// M78 (MA-A1): 게임 신규 실행/앱 첫 기동 시 어떤 모드를 적용할지 결정한다.
// 활성 자동 규칙이 있으면 그 모드 우선(자동 60초 타이머가 깨우기 전 ~60초 윈도우에
// last_user_mode가 자동 규칙을 덮어쓰는 회귀 차단), 없으면 last_user_mode를 사용.
// pure fn으로 분리해 단위 테스트로 우선순위 invariant 고정.
pub(crate) fn pick_initial_mode(
    active_rule_mode: Option<schedule::OptimizeMode>,
    last_user_mode: Option<schedule::OptimizeMode>,
) -> Option<schedule::OptimizeMode> {
    active_rule_mode.or(last_user_mode)
}

// AppState/settings 의존 wrapper. apply_initial / status_timer toggle 분기에서 호출.
fn resolve_initial_mode(state: &AppState) -> Option<schedule::OptimizeMode> {
    let active = schedule::active_rule(&state.schedule.rules.borrow()).map(|r| r.mode);
    let last_user = settings::load_settings().last_user_mode;
    pick_initial_mode(active, last_user)
}

// 게임가드 우회: 적용 직후 0.5/1/2/5/10초에 동일 모드 재적용 5회.
// 이전 timer 벡터는 새 호출 시점에 drop되어 자연 취소된다.
fn schedule_reapply(app: &AppWindow, state: &AppState, mode: schedule::OptimizeMode) {
    let delays_ms: [u64; 5] = [500, 1000, 2000, 5000, 10000];
    let mut new_timers: Vec<slint::Timer> = Vec::with_capacity(delays_ms.len());
    for ms in delays_ms {
        let t = slint::Timer::default();
        let app_w = app.as_weak();
        let mode_c = mode;
        t.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(ms),
            move || {
                if let Some(app) = app_w.upgrade() {
                    let info = process::get_cpu_info();
                    let (affinity, priority, _, _) = mode_params(&mode_c, &info);
                    if let Some(pid) = process::find_process_id("BlackDesert64.exe") {
                        // 실패는 무음 (PID TOCTOU에서 거부되는 정상 경로 포함). 사용자 피드백은 1차 적용에서만.
                        let _ = process::apply_optimization(pid, affinity, priority);
                        let _ = &app;
                    }
                }
            },
        );
        new_timers.push(t);
    }
    *state.mode.reapply_timers.borrow_mut() = new_timers;
}

fn should_refresh_shutdown_ui(tick: &mut u32) -> bool {
    let should_refresh = *tick == 0 || *tick >= 12;
    *tick = if should_refresh { 1 } else { *tick + 1 };
    should_refresh
}

pub fn apply_mode(app: &AppWindow, state: &AppState, mode: schedule::OptimizeMode) {
    let info = process::get_cpu_info();
    let (affinity, priority, success_text, _mode_text) = mode_params(&mode, &info);
    match process::find_process_id("BlackDesert64.exe") {
        Some(pid) => match process::apply_optimization(pid, affinity, priority) {
            Ok(()) => {
                tracing::info!(
                    pid,
                    mode = ?mode,
                    affinity = format_args!("{:#x}", affinity),
                    hybrid = info.has_hybrid,
                    "mode applied"
                );
                // 모드 적용은 이전 모드 적용 메시지를 의미 없게 만들므로 prev를 비우면서 갱신.
                app.set_prev_status_text("".into());
                app.set_status_text(success_text.into());
                set_current_mode_state(app, state, Some(mode));
                if !app.get_reduce_motion() {
                    app.set_mode_ripple(1.0);
                }
                schedule_reapply(app, state, mode);
            }
            Err(e) => {
                tracing::error!(pid, mode = ?mode, error = %e, "apply_optimization failed");
                push_status(app, format!("오류: {e}"));
            }
        },
        None => {
            tracing::warn!(mode = ?mode, "mode apply skipped: game process not found");
            push_status(app, "BlackDesert64.exe 프로세스를 찾을 수 없습니다.");
        }
    }
}

pub fn apply_initial(app: &AppWindow, state: &AppState) {
    app.set_admin_ok(admin::is_admin());
    let game_running = launcher::is_game_running();
    app.set_game_running(game_running);
    *state.game.last_game_state.borrow_mut() = game_running;
    let current_mode = if game_running {
        process::find_process_id("BlackDesert64.exe").and_then(process::query_current_mode)
    } else {
        None
    };
    set_current_mode_state(app, state, current_mode);

    if game_running {
        if let Some(pid) = process::find_process_id("BlackDesert64.exe") {
            match fps::FpsSession::start(pid) {
                Ok(s) => *state.game.fps_session.borrow_mut() = Some(s),
                Err(e) => {
                    tracing::warn!(error = %e, "fps session start failed");
                    push_status(app, format!("FPS 측정 실패: {e}"));
                }
            }
        }
        // M50/M78: 앱 첫 기동 시 게임 이미 실행 중이면 우선순위에 따라 모드 자동 적용.
        // 활성 자동 규칙이 있으면 그 모드, 없으면 last_user_mode.
        if let Some(m) = resolve_initial_mode(state) {
            apply_mode(app, state, m);
        }
    }
}

pub fn register(app: &AppWindow, state: &AppState) {
    app.on_apply_high_mode({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                apply_user_mode(&app, &state, schedule::OptimizeMode::High);
            }
        }
    });

    app.on_apply_normal_mode({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                apply_user_mode(&app, &state, schedule::OptimizeMode::Normal);
            }
        }
    });

    app.on_apply_low_power_mode({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                apply_user_mode(&app, &state, schedule::OptimizeMode::LowPower);
            }
        }
    });

    app.on_launch_game({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let user_path = app.get_launcher_path().to_string();
                if user_path.chars().count() > 512 {
                    push_status(&app, "런처 경로가 너무 깁니다. 512자 이내로 입력하세요.");
                    return;
                }
                let msg = match launcher::launch_game(&user_path) {
                    launcher::LaunchResult::GameAlreadyRunning => {
                        "게임이 이미 실행 중입니다. 런처를 실행하지 않습니다.".to_string()
                    }
                    launcher::LaunchResult::LauncherStarted(path) => {
                        format!("런처 실행됨: {}", path.display())
                    }
                    launcher::LaunchResult::LauncherNotFound => {
                        "런처를 찾을 수 없습니다. 경로를 직접 입력하세요.".to_string()
                    }
                };
                push_status(&app, msg);
                app.set_game_running(launcher::is_game_running());
            }
        }
    });

    app.on_refresh_game_status({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                let running = launcher::is_game_running();
                *state.game.last_game_state.borrow_mut() = running;
                app.set_game_running(running);
                app.set_admin_ok(admin::is_admin());
                let current_mode = if running {
                    process::find_process_id("BlackDesert64.exe")
                        .and_then(process::query_current_mode)
                } else {
                    None
                };
                set_current_mode_state(&app, &state, current_mode);
                let msg = if running {
                    "게임 실행 중 (BlackDesert64.exe 확인됨)."
                } else {
                    "게임 미실행 상태."
                };
                push_status(&app, msg);
            }
        }
    });
}

pub fn start_status_timer(app: &AppWindow, state: &AppState) -> slint::Timer {
    let timer = slint::Timer::default();
    let mut shutdown_refresh_tick = 0u32;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(5),
        {
            let app = app.as_weak();
            let state = state.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    let pid = process::find_process_id("BlackDesert64.exe");
                    let running = pid.is_some();
                    let prev = *state.game.last_game_state.borrow();
                    *state.game.last_game_state.borrow_mut() = running;
                    app.set_game_running(running);
                    if running != prev {
                        if running {
                            push_status(&app, "게임 프로세스가 감지되었습니다.");
                            if let Some(p) = pid {
                                if state.game.fps_session.borrow().is_none() {
                                    match fps::FpsSession::start(p) {
                                        Ok(s) => *state.game.fps_session.borrow_mut() = Some(s),
                                        Err(e) => {
                                            eprintln!("[fps] {e}");
                                            push_status(&app, format!("FPS 측정 실패: {e}"));
                                        }
                                    }
                                }
                            }
                            // M50/M78: 게임 신규 실행 감지 시 우선순위에 따라 모드 적용.
                            // 활성 자동 규칙이 있으면 그 모드(자동 60초 타이머가 깨우기 전에
                            // 의도된 모드 즉시 반영), 없으면 last_user_mode.
                            if let Some(m) = resolve_initial_mode(&state) {
                                apply_mode(&app, &state, m);
                            }
                        } else {
                            push_status(&app, "게임 프로세스가 종료되었습니다.");
                            set_current_mode_state(&app, &state, None);
                            *state.game.fps_session.borrow_mut() = None;
                            app.set_mon_fps("--".into());
                            *state.game.prev_game_visible.borrow_mut() = true;
                        }
                    }
                    if should_refresh_shutdown_ui(&mut shutdown_refresh_tick) {
                        crate::ui::shutdown::refresh_shutdown_ui(&app);
                    }

                    if let Some(pid) = pid {
                        let current_mode = process::query_current_mode(pid);
                        set_current_mode_state(&app, &state, current_mode);

                        // 자동 저전력 모드: 게임 창 가시성 토글에 따라 저전력↔직전 모드 자동 전환.
                        if app.get_auto_tray_on_game_minimize() {
                            if let Some(hwnd) = window::find_main_window(pid) {
                                let visible = window::is_visible(hwnd);
                                let prev_visible = *state.game.prev_game_visible.borrow();
                                *state.game.prev_game_visible.borrow_mut() = visible;
                                if prev_visible && !visible {
                                    // 사라짐: 현재 사용자 모드 백업(LowPower 자체는 제외) 후 저전력 적용.
                                    // M78: query_current_mode가 enum 직접 반환이라 string 라운드트립 제거.
                                    if let Some(m) = current_mode {
                                        if m != schedule::OptimizeMode::LowPower {
                                            *state.mode.last_user_mode.borrow_mut() = Some(m);
                                        }
                                    }
                                    apply_mode(&app, &state, schedule::OptimizeMode::LowPower);
                                } else if !prev_visible && visible {
                                    // 다시 보임: 백업한 모드로 복원.
                                    let restore = *state.mode.last_user_mode.borrow();
                                    if let Some(m) = restore {
                                        apply_mode(&app, &state, m);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    use super::*;
    use schedule::OptimizeMode;

    #[test]
    fn pick_initial_mode_prefers_active_rule_over_last_user() {
        assert_eq!(
            pick_initial_mode(Some(OptimizeMode::LowPower), Some(OptimizeMode::High)),
            Some(OptimizeMode::LowPower)
        );
    }

    #[test]
    fn pick_initial_mode_falls_back_to_last_user_when_no_active_rule() {
        assert_eq!(
            pick_initial_mode(None, Some(OptimizeMode::Normal)),
            Some(OptimizeMode::Normal)
        );
    }

    #[test]
    fn pick_initial_mode_returns_none_when_both_missing() {
        assert_eq!(pick_initial_mode(None, None), None);
    }

    #[test]
    fn mode_label_is_stable_for_each_variant() {
        assert_eq!(mode_label(OptimizeMode::High), "고성능 모드");
        assert_eq!(mode_label(OptimizeMode::Normal), "일반 모드");
        assert_eq!(mode_label(OptimizeMode::LowPower), "저전력 모드");
    }

    #[test]
    fn current_mode_helper_updates_tray_indicator() {
        let src = include_str!("control.rs");
        assert!(src.contains("fn set_current_mode_state("));
        assert!(src.contains("tray_handle"));
        assert!(src.contains("set_mode_indicator"));
        assert!(!src.contains(concat!("fn ", "set_app_current_mode(")));
    }

    #[test]
    fn shutdown_refresh_polling_runs_first_tick_then_every_sixty_seconds() {
        let mut tick = 0;
        assert!(should_refresh_shutdown_ui(&mut tick));
        for _ in 0..11 {
            assert!(!should_refresh_shutdown_ui(&mut tick));
        }
        assert!(should_refresh_shutdown_ui(&mut tick));
    }
}
