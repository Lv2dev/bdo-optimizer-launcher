use crate::backend::{process, schedule, settings, shutdown, tray, window};
use crate::tauri_commands;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::thread;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WindowEvent};

const MAIN_WINDOW_LABEL: &str = "main";
const MENU_TOGGLE_WINDOW: &str = "tray_toggle_window";
const MENU_APPLY_HIGH: &str = "tray_apply_high";
const MENU_APPLY_NORMAL: &str = "tray_apply_normal";
const MENU_APPLY_LOW_POWER: &str = "tray_apply_low_power";
const MENU_CANCEL_SHUTDOWN: &str = "tray_cancel_shutdown";
const MENU_QUIT: &str = "tray_quit";
const TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/tray_16.png");

type TauriMenuItem = MenuItem<tauri::Wry>;
type TauriTrayIcon = TrayIcon<tauri::Wry>;

#[derive(Debug, Clone, Copy, PartialEq)]
enum CloseRequestAction {
    HideToTray,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrayLifecycleCommand {
    ToggleWindow,
    ApplyMode(schedule::OptimizeMode),
    CancelShutdown,
    Quit,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum AutoLowPowerAction {
    Noop,
    ApplyLowPower,
    Restore(schedule::OptimizeMode),
}

struct LifecycleState {
    tray_icon: TauriTrayIcon,
    toggle_item: TauriMenuItem,
    high_item: TauriMenuItem,
    normal_item: TauriMenuItem,
    low_power_item: TauriMenuItem,
    quitting: AtomicBool,
    automation_state: Mutex<AutomationState>,
}

#[derive(Debug, Default)]
struct AutomationState {
    previous_present: Option<bool>,
    previous_visible: Option<bool>,
    auto_restore_mode: Option<schedule::OptimizeMode>,
    active_schedule_id: Option<u32>,
    schedule_restore_mode: Option<schedule::OptimizeMode>,
}

#[derive(Clone, Copy)]
struct AutomationInput {
    current_present: bool,
    current_visible: Option<bool>,
    current_mode: Option<schedule::OptimizeMode>,
    auto_low_power: bool,
    active_schedule: Option<(u32, schedule::OptimizeMode)>,
    last_user_mode: Option<schedule::OptimizeMode>,
    default_mode: Option<schedule::OptimizeMode>,
}

fn automation_mode_action(
    state: &mut AutomationState,
    input: AutomationInput,
) -> Option<schedule::OptimizeMode> {
    let newly_present = matches!(
        (state.previous_present, input.current_present),
        (None, true) | (Some(false), true)
    );
    state.previous_present = Some(input.current_present);

    if !input.current_present {
        state.previous_visible = None;
        state.auto_restore_mode = None;
        state.active_schedule_id = None;
        state.schedule_restore_mode = None;
        return None;
    }

    let previous_visible = state.previous_visible;
    state.previous_visible = input.current_visible;

    if input.current_visible == Some(false) {
        if input.auto_low_power {
            if previous_visible != Some(false)
                && input.current_mode != Some(schedule::OptimizeMode::LowPower)
            {
                state.auto_restore_mode = input.current_mode;
            }
            if input.current_mode != Some(schedule::OptimizeMode::LowPower) {
                return Some(schedule::OptimizeMode::LowPower);
            }
        } else if newly_present {
            return input
                .default_mode
                .filter(|mode| Some(*mode) != input.current_mode);
        }
        return None;
    }

    if input.current_visible != Some(true) {
        return None;
    }

    let desired = if let Some((rule_id, mode)) = input.active_schedule {
        if state.active_schedule_id.is_none() {
            state.schedule_restore_mode = input
                .last_user_mode
                .or(input.default_mode)
                .or(state.auto_restore_mode)
                .or(input.current_mode);
        }
        state.active_schedule_id = Some(rule_id);
        Some(mode)
    } else {
        let schedule_exited = state.active_schedule_id.take().is_some();
        if schedule_exited {
            input
                .last_user_mode
                .or(input.default_mode)
                .or(state.auto_restore_mode.take())
                .or(state.schedule_restore_mode.take())
        } else if previous_visible == Some(false) && input.auto_low_power {
            input
                .last_user_mode
                .or(input.default_mode)
                .or(state.auto_restore_mode.take())
        } else if input.last_user_mode.is_some() {
            input.last_user_mode
        } else if newly_present {
            input.default_mode
        } else {
            None
        }
    };

    desired.filter(|mode| Some(*mode) != input.current_mode)
}

impl LifecycleState {
    fn set_toggle_label(&self, window_visible: bool) {
        let label = if window_visible {
            "창 숨기기"
        } else {
            "창 열기"
        };
        let _ = self.toggle_item.set_text(label);
    }

    fn set_mode_indicator(&self, mode: Option<schedule::OptimizeMode>) {
        let (high, normal, low_power) = tray::mode_menu_labels(mode);
        let _ = self.high_item.set_text(high);
        let _ = self.normal_item.set_text(normal);
        let _ = self.low_power_item.set_text(low_power);
        let _ = self.tray_icon.set_tooltip(Some(tray::mode_tooltip(mode)));
    }
}

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let app_handle = app.handle().clone();
    match build_tray(&app_handle) {
        Ok(state) => {
            let _ = app_handle.manage(state);
            sync_tray_mode(&app_handle, current_game_mode());
            if start_minimized_requested(std::env::args()) {
                hide_main_window(&app_handle);
            }
            start_auto_low_power_worker(app_handle.clone());
        }
        Err(error) => {
            tracing::warn!(error = %error, "tauri tray init failed");
        }
    }
    register_window_close_handler(&app_handle);
    Ok(())
}

pub(crate) fn sync_tray_mode(app: &AppHandle, mode: Option<schedule::OptimizeMode>) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.set_mode_indicator(mode);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<LifecycleState> {
    let menu = Menu::new(app)?;

    let toggle_item = MenuItem::with_id(app, MENU_TOGGLE_WINDOW, "창 숨기기", true, None::<&str>)?;
    let high_item =
        MenuItem::with_id(app, MENU_APPLY_HIGH, "고성능 모드 적용", true, None::<&str>)?;
    let normal_item =
        MenuItem::with_id(app, MENU_APPLY_NORMAL, "일반 모드 적용", true, None::<&str>)?;
    let low_power_item = MenuItem::with_id(
        app,
        MENU_APPLY_LOW_POWER,
        "저전력 모드 적용",
        true,
        None::<&str>,
    )?;
    let cancel_item = MenuItem::with_id(
        app,
        MENU_CANCEL_SHUTDOWN,
        "예약 종료 취소",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "종료", true, None::<&str>)?;

    menu.append(&toggle_item)?;
    menu.append(&high_item)?;
    menu.append(&normal_item)?;
    menu.append(&low_power_item)?;
    menu.append(&cancel_item)?;
    menu.append(&separator)?;
    menu.append(&quit_item)?;

    let icon = Image::from_bytes(TRAY_ICON_PNG)?;
    let tray_icon = TrayIconBuilder::new()
        .icon(icon)
        .tooltip(tray::mode_tooltip(None))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if let Some(command) = tray_command_from_menu_id(event.id().0.as_str()) {
                handle_tray_command(app, command);
            }
        })
        .on_tray_icon_event(|tray_icon, event| {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                show_main_window(tray_icon.app_handle());
            }
        })
        .build(app)?;

    Ok(LifecycleState {
        tray_icon,
        toggle_item,
        high_item,
        normal_item,
        low_power_item,
        quitting: AtomicBool::new(false),
        automation_state: Mutex::new(AutomationState::default()),
    })
}

fn register_window_close_handler(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        tracing::warn!("main webview window not found; close-to-tray disabled");
        return;
    };
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let tray_available = app_handle.try_state::<LifecycleState>().is_some();
            let close_to_tray = tray_available && settings::load_settings().close_to_tray;
            let quitting = app_handle
                .try_state::<LifecycleState>()
                .map(|state| state.quitting.load(Ordering::SeqCst))
                .unwrap_or(false);

            if close_request_action(close_to_tray, quitting) == CloseRequestAction::HideToTray {
                api.prevent_close();
                hide_main_window(&app_handle);
            }
        }
    });
}

fn handle_tray_command(app: &AppHandle, command: TrayLifecycleCommand) {
    match command {
        TrayLifecycleCommand::ToggleWindow => toggle_main_window(app),
        TrayLifecycleCommand::ApplyMode(mode) => {
            let response = tauri_commands::apply_mode_for_lifecycle(mode, true);
            sync_tray_mode(
                app,
                response
                    .control
                    .current_mode
                    .map(schedule::OptimizeMode::from),
            );
        }
        TrayLifecycleCommand::CancelShutdown => {
            let message = match cancel_all_shutdowns(shutdown::cancel_once, shutdown::cancel_weekly)
            {
                Ok(message) => message,
                Err(error) => format!("오류: {error}"),
            };
            tracing::info!(message, "tray cancel shutdown requested");
        }
        TrayLifecycleCommand::Quit => request_quit(app),
    }
}

fn cancel_all_shutdowns<F, G>(cancel_once: F, cancel_weekly: G) -> Result<String, String>
where
    F: FnOnce() -> Result<(), shutdown::Error>,
    G: FnOnce() -> Result<(), shutdown::Error>,
{
    let mut errors = Vec::new();
    if let Err(error) = cancel_once() {
        if !matches!(error, shutdown::Error::TaskNotFound) {
            errors.push(format!("단발 예약: {error}"));
        }
    }
    if let Err(error) = cancel_weekly() {
        if !matches!(error, shutdown::Error::TaskNotFound) {
            errors.push(format!("매주 예약: {error}"));
        }
    }

    if errors.is_empty() {
        Ok("단발 및 매주 종료 예약이 취소되었습니다.".to_string())
    } else {
        Err(errors.join(" / "))
    }
}

fn close_request_action(close_to_tray: bool, quitting: bool) -> CloseRequestAction {
    if close_to_tray && !quitting {
        CloseRequestAction::HideToTray
    } else {
        CloseRequestAction::Exit
    }
}

fn tray_command_from_menu_id(id: &str) -> Option<TrayLifecycleCommand> {
    match id {
        MENU_TOGGLE_WINDOW => Some(TrayLifecycleCommand::ToggleWindow),
        MENU_APPLY_HIGH => Some(TrayLifecycleCommand::ApplyMode(
            schedule::OptimizeMode::High,
        )),
        MENU_APPLY_NORMAL => Some(TrayLifecycleCommand::ApplyMode(
            schedule::OptimizeMode::Normal,
        )),
        MENU_APPLY_LOW_POWER => Some(TrayLifecycleCommand::ApplyMode(
            schedule::OptimizeMode::LowPower,
        )),
        MENU_CANCEL_SHUTDOWN => Some(TrayLifecycleCommand::CancelShutdown),
        MENU_QUIT => Some(TrayLifecycleCommand::Quit),
        _ => None,
    }
}

#[cfg(test)]
fn auto_low_power_transition(
    enabled: bool,
    previous_visible: Option<bool>,
    current_visible: Option<bool>,
    restore_mode: Option<schedule::OptimizeMode>,
) -> AutoLowPowerAction {
    if !enabled {
        return AutoLowPowerAction::Noop;
    }
    match (previous_visible, current_visible) {
        (None, Some(false)) => AutoLowPowerAction::ApplyLowPower,
        (Some(true), Some(false)) => AutoLowPowerAction::ApplyLowPower,
        (Some(false), Some(true)) => restore_mode
            .map(AutoLowPowerAction::Restore)
            .unwrap_or(AutoLowPowerAction::Noop),
        _ => AutoLowPowerAction::Noop,
    }
}

#[cfg(test)]
fn visible_mode_maintenance_action(
    current_visible: Option<bool>,
    desired_mode: Option<schedule::OptimizeMode>,
    current_mode: Option<schedule::OptimizeMode>,
) -> Option<schedule::OptimizeMode> {
    if current_visible != Some(true) {
        return None;
    }
    let desired = desired_mode?;
    if current_mode == Some(desired) {
        None
    } else {
        Some(desired)
    }
}

fn start_minimized_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|arg| arg.as_ref() == "--minimized")
}

fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if visible {
        hide_main_window(app);
    } else {
        show_main_window(app);
    }
}

fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = window.show();
    let _ = window.set_focus();
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.set_toggle_label(true);
    }
}

fn hide_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = window.hide();
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.set_toggle_label(false);
    }
}

fn request_quit(app: &AppHandle) {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.quitting.store(true, Ordering::SeqCst);
    }
    app.exit(0);
}

#[derive(Clone, Copy)]
struct GameWindowState {
    pid: u32,
    visible: bool,
}

fn query_game_window_state() -> Option<GameWindowState> {
    let pid = process::find_process_id("BlackDesert64.exe")?;
    let hwnd = window::find_main_window(pid)?;
    Some(GameWindowState {
        pid,
        visible: window::is_visible(hwnd),
    })
}

fn current_game_mode() -> Option<schedule::OptimizeMode> {
    process::find_process_id("BlackDesert64.exe").and_then(process::query_current_mode)
}

fn log_game_mode_diagnostics(game_state: Option<GameWindowState>) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    let Some(game_state) = game_state else {
        return;
    };
    let info = process::get_cpu_info();
    let snapshot = process::query_process_mode_snapshot(game_state.pid);
    match snapshot {
        Some(snapshot) => {
            tracing::debug!(
                pid = game_state.pid,
                foreground_pid = ?window::foreground_process_id(),
                visible = game_state.visible,
                priority_class = snapshot.priority_class,
                affinity = format_args!("{:#x}", snapshot.affinity_mask),
                expected_high = format_args!("{:#x}", process::calc_high_affinity(&info)),
                expected_normal = format_args!("{:#x}", process::calc_normal_affinity(&info)),
                expected_low_power = format_args!("{:#x}", process::calc_low_power_affinity(&info)),
                "game mode diagnostic tick"
            );
        }
        None => {
            tracing::debug!(
                pid = game_state.pid,
                foreground_pid = ?window::foreground_process_id(),
                visible = game_state.visible,
                "game mode diagnostic tick unavailable"
            );
        }
    }
}

fn start_auto_low_power_worker(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));
        if app
            .try_state::<LifecycleState>()
            .map(|state| state.quitting.load(Ordering::SeqCst))
            .unwrap_or(true)
        {
            break;
        }
        run_auto_low_power_tick(&app);
    });
}

// M96 P3: default_mode 자동 적용 판정. 게임이 (없음|첫 tick)에서 감지로 전환될 때만 적용한다.
#[cfg(test)]
fn default_mode_action(
    default_mode: Option<schedule::OptimizeMode>,
    previous_present: Option<bool>,
    current_present: bool,
) -> Option<schedule::OptimizeMode> {
    let mode = default_mode?;
    match (previous_present, current_present) {
        (None, true) | (Some(false), true) => Some(mode),
        _ => None,
    }
}

fn run_auto_low_power_tick(app: &AppHandle) {
    let Some(state) = app.try_state::<LifecycleState>() else {
        return;
    };
    let response = tauri_commands::apply_mode_for_lifecycle_decision(|| {
        let setting = settings::load_settings();
        let game_state = query_game_window_state();
        log_game_mode_diagnostics(game_state);
        let current_mode = game_state.and_then(|game| process::query_current_mode(game.pid));
        let rules = schedule::load_rules();
        let active_schedule = schedule::active_rule(&rules).map(|rule| (rule.id, rule.mode));
        let input = AutomationInput {
            current_present: game_state.is_some(),
            current_visible: game_state.map(|game| game.visible),
            current_mode,
            auto_low_power: setting.auto_tray_on_game_minimize,
            active_schedule,
            last_user_mode: setting.last_user_mode,
            default_mode: setting.default_mode,
        };
        let mut automation = state
            .automation_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        automation_mode_action(&mut automation, input)
    });

    if let Some(response) = response {
        sync_tray_mode(
            app,
            response
                .control
                .current_mode
                .map(schedule::OptimizeMode::from),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::schedule::OptimizeMode;
    use std::cell::Cell;

    #[test]
    fn close_to_tray_hides_window_unless_quitting() {
        assert_eq!(
            close_request_action(true, false),
            CloseRequestAction::HideToTray
        );
        assert_eq!(close_request_action(false, false), CloseRequestAction::Exit);
        assert_eq!(close_request_action(true, true), CloseRequestAction::Exit);
    }

    #[test]
    fn tray_menu_id_maps_to_lifecycle_command() {
        assert_eq!(
            tray_command_from_menu_id("tray_toggle_window"),
            Some(TrayLifecycleCommand::ToggleWindow)
        );
        assert_eq!(
            tray_command_from_menu_id("tray_apply_low_power"),
            Some(TrayLifecycleCommand::ApplyMode(OptimizeMode::LowPower))
        );
        assert_eq!(tray_command_from_menu_id("tray_unknown"), None);
    }

    #[test]
    fn tray_cancel_handles_once_only_schedule() {
        let once_calls = Cell::new(0);
        let weekly_calls = Cell::new(0);

        let result = cancel_all_shutdowns(
            || {
                once_calls.set(once_calls.get() + 1);
                Ok(())
            },
            || {
                weekly_calls.set(weekly_calls.get() + 1);
                Err(shutdown::Error::TaskNotFound)
            },
        );

        assert!(result.is_ok());
        assert_eq!((once_calls.get(), weekly_calls.get()), (1, 1));
    }

    #[test]
    fn tray_cancel_handles_weekly_only_schedule() {
        let once_calls = Cell::new(0);
        let weekly_calls = Cell::new(0);

        let result = cancel_all_shutdowns(
            || {
                once_calls.set(once_calls.get() + 1);
                Err(shutdown::Error::TaskNotFound)
            },
            || {
                weekly_calls.set(weekly_calls.get() + 1);
                Ok(())
            },
        );

        assert!(result.is_ok());
        assert_eq!((once_calls.get(), weekly_calls.get()), (1, 1));
    }

    #[test]
    fn tray_cancel_handles_both_schedules_and_aggregates_real_errors() {
        let success = cancel_all_shutdowns(|| Ok(()), || Ok(()));
        assert!(success.is_ok());

        let failure = cancel_all_shutdowns(
            || Err(shutdown::Error::DeleteFailed("once".to_string())),
            || Err(shutdown::Error::DeleteFailed("weekly".to_string())),
        )
        .unwrap_err();
        assert!(failure.contains("단발"));
        assert!(failure.contains("매주"));
        assert!(failure.contains("once"));
        assert!(failure.contains("weekly"));
    }

    #[test]
    fn auto_low_power_only_runs_on_visible_to_hidden_transition() {
        assert_eq!(
            auto_low_power_transition(true, Some(true), Some(false), None),
            AutoLowPowerAction::ApplyLowPower
        );
        assert_eq!(
            auto_low_power_transition(true, None, Some(false), None),
            AutoLowPowerAction::ApplyLowPower
        );
        assert_eq!(
            auto_low_power_transition(true, None, Some(true), None),
            AutoLowPowerAction::Noop
        );
        assert_eq!(
            auto_low_power_transition(true, Some(false), Some(false), None),
            AutoLowPowerAction::Noop
        );
        assert_eq!(
            auto_low_power_transition(false, Some(true), Some(false), None),
            AutoLowPowerAction::Noop
        );
    }

    #[test]
    fn auto_low_power_restores_saved_mode_on_hidden_to_visible_transition() {
        assert_eq!(
            auto_low_power_transition(true, Some(false), Some(true), Some(OptimizeMode::Normal)),
            AutoLowPowerAction::Restore(OptimizeMode::Normal)
        );
        assert_eq!(
            auto_low_power_transition(true, Some(false), Some(true), None),
            AutoLowPowerAction::Noop
        );
    }

    #[test]
    fn visible_mode_maintenance_reapplies_last_user_mode_only_when_visible() {
        use OptimizeMode::{High, LowPower, Normal};

        assert_eq!(
            visible_mode_maintenance_action(Some(true), Some(High), Some(Normal)),
            Some(High)
        );
        assert_eq!(
            visible_mode_maintenance_action(Some(true), Some(High), None),
            Some(High)
        );
        assert_eq!(
            visible_mode_maintenance_action(Some(true), Some(High), Some(High)),
            None
        );
        assert_eq!(
            visible_mode_maintenance_action(Some(false), Some(High), Some(LowPower)),
            None
        );
        assert_eq!(
            visible_mode_maintenance_action(None, Some(High), Some(Normal)),
            None
        );
        assert_eq!(
            visible_mode_maintenance_action(Some(true), None, Some(Normal)),
            None
        );
    }

    #[test]
    fn start_minimized_arg_is_detected_for_autostart_tray_launch() {
        assert!(start_minimized_requested(["app.exe", "--minimized"]));
        assert!(!start_minimized_requested(["app.exe"]));
    }

    #[test]
    fn default_mode_applies_only_when_game_newly_appears() {
        use OptimizeMode::High;
        // 게임이 없다가/첫 tick에 감지 → 적용
        assert_eq!(
            default_mode_action(Some(High), Some(false), true),
            Some(High)
        );
        assert_eq!(default_mode_action(Some(High), None, true), Some(High));
        // 연속 실행 중(이미 적용) → 재적용 안 함
        assert_eq!(default_mode_action(Some(High), Some(true), true), None);
        // 게임 종료 / 미실행 → 적용 안 함
        assert_eq!(default_mode_action(Some(High), Some(true), false), None);
        assert_eq!(default_mode_action(Some(High), None, false), None);
        // default_mode 없음(수동) → 적용 안 함
        assert_eq!(default_mode_action(None, Some(false), true), None);
    }

    fn automation_input(
        visible: Option<bool>,
        current_mode: Option<OptimizeMode>,
    ) -> AutomationInput {
        AutomationInput {
            current_present: true,
            current_visible: visible,
            current_mode,
            auto_low_power: true,
            active_schedule: None,
            last_user_mode: None,
            default_mode: None,
        }
    }

    #[test]
    fn automation_priority_is_hidden_then_schedule_then_user_then_default() {
        let mut state = AutomationState::default();
        let mut input = automation_input(Some(true), Some(OptimizeMode::Normal));
        input.active_schedule = Some((7, OptimizeMode::High));
        input.last_user_mode = Some(OptimizeMode::Normal);
        input.default_mode = Some(OptimizeMode::LowPower);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::High)
        );

        input.current_visible = Some(false);
        input.current_mode = Some(OptimizeMode::High);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::LowPower)
        );

        input.current_visible = Some(true);
        input.current_mode = Some(OptimizeMode::LowPower);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::High)
        );

        input.active_schedule = None;
        input.current_mode = Some(OptimizeMode::High);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::Normal)
        );
    }

    #[test]
    fn automation_applies_default_only_when_game_first_appears() {
        let mut state = AutomationState::default();
        let mut input = automation_input(Some(true), Some(OptimizeMode::Normal));
        input.default_mode = Some(OptimizeMode::High);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::High)
        );

        input.current_mode = Some(OptimizeMode::Normal);
        assert_eq!(automation_mode_action(&mut state, input), None);
    }

    #[test]
    fn automation_first_hidden_tick_enters_low_power_and_restores_baseline() {
        let mut state = AutomationState::default();
        let mut input = automation_input(Some(false), Some(OptimizeMode::High));
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::LowPower)
        );

        input.current_visible = Some(true);
        input.current_mode = Some(OptimizeMode::LowPower);
        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::High)
        );
    }

    #[test]
    fn automation_first_hidden_tick_applies_default_when_auto_low_power_is_disabled() {
        let mut state = AutomationState::default();
        let mut input = automation_input(Some(false), Some(OptimizeMode::Normal));
        input.auto_low_power = false;
        input.default_mode = Some(OptimizeMode::High);

        assert_eq!(
            automation_mode_action(&mut state, input),
            Some(OptimizeMode::High)
        );
    }
}
