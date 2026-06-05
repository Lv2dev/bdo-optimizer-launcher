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
    previous_game_window_visible: Mutex<Option<bool>>,
    // M96 P3: 직전 tick의 게임 프로세스 존재 여부. 신규 등장 시 default_mode 자동 적용.
    previous_game_present: Mutex<Option<bool>>,
    auto_restore_mode: Mutex<Option<schedule::OptimizeMode>>,
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
        previous_game_window_visible: Mutex::new(None),
        previous_game_present: Mutex::new(None),
        auto_restore_mode: Mutex::new(None),
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
            let message = match shutdown::cancel_once() {
                Ok(()) => "단발 종료 예약이 취소되었습니다.".to_string(),
                Err(error) => format!("오류: {error}"),
            };
            tracing::info!(message, "tray cancel shutdown requested");
        }
        TrayLifecycleCommand::Quit => request_quit(app),
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

fn apply_visible_mode_maintenance(
    app: &AppHandle,
    current_visible: Option<bool>,
    desired_mode: Option<schedule::OptimizeMode>,
) {
    if let Some(mode) =
        visible_mode_maintenance_action(current_visible, desired_mode, current_game_mode())
    {
        let response = tauri_commands::apply_mode_for_lifecycle(mode, false);
        sync_tray_mode(
            app,
            response
                .control
                .current_mode
                .map(schedule::OptimizeMode::from),
        );
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

// 게임 신규 실행을 감지하면 default_mode를 적용한다.
// 자동 적용이므로 last_user_mode는 갱신하지 않는다(persist=false).
fn apply_default_mode_on_game_launch(
    app: &AppHandle,
    state: &LifecycleState,
    default_mode: Option<schedule::OptimizeMode>,
) {
    let current_present = process::find_process_id("BlackDesert64.exe").is_some();
    // read-modify-write를 단일 lock 스코프로 묶는다(이전 값 읽기와 새 값 저장 사이에 lock을
    // 두 번 따로 잡으면 그 틈에 상태가 바뀌어 transition 판정이 어긋날 수 있다).
    let previous_present = {
        let mut guard = state
            .previous_game_present
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = *guard;
        *guard = Some(current_present);
        previous
    };

    if let Some(mode) = default_mode_action(default_mode, previous_present, current_present) {
        let response = tauri_commands::apply_mode_for_lifecycle(mode, false);
        sync_tray_mode(
            app,
            response
                .control
                .current_mode
                .map(schedule::OptimizeMode::from),
        );
    }
}

fn run_auto_low_power_tick(app: &AppHandle) {
    let Some(state) = app.try_state::<LifecycleState>() else {
        return;
    };
    let setting = settings::load_settings();

    // M96 P3: 게임 신규 등장 시 기본 모드 자동 적용(auto_tray_on_game_minimize 게이트와 독립).
    apply_default_mode_on_game_launch(app, &state, setting.default_mode);

    let game_state = query_game_window_state();
    let current_visible = game_state.map(|state| state.visible);
    log_game_mode_diagnostics(game_state);

    // 자동 저전력 OFF면 hidden/visible 전환은 처리하지 않는다. visible 상태 유지 판단은 계속
    // 수행하되, 다음 ON 전환에서 오탐 transition이 생기지 않도록 직전 visible 상태만 초기화한다.
    if !setting.auto_tray_on_game_minimize {
        *state
            .previous_game_window_visible
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        apply_visible_mode_maintenance(app, current_visible, setting.last_user_mode);
        return;
    }
    let previous_visible = *state
        .previous_game_window_visible
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let restore_mode = *state
        .auto_restore_mode
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let action = auto_low_power_transition(
        setting.auto_tray_on_game_minimize,
        previous_visible,
        current_visible,
        restore_mode,
    );

    *state
        .previous_game_window_visible
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = current_visible;

    match action {
        AutoLowPowerAction::Noop => {
            apply_visible_mode_maintenance(app, current_visible, setting.last_user_mode);
        }
        AutoLowPowerAction::ApplyLowPower => {
            if let Some(game_state) = game_state {
                if let Some(mode) = process::query_current_mode(game_state.pid) {
                    if mode != schedule::OptimizeMode::LowPower {
                        *state
                            .auto_restore_mode
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(mode);
                    }
                }
            }
            let response =
                tauri_commands::apply_mode_for_lifecycle(schedule::OptimizeMode::LowPower, false);
            sync_tray_mode(
                app,
                response
                    .control
                    .current_mode
                    .map(schedule::OptimizeMode::from),
            );
        }
        AutoLowPowerAction::Restore(mode) => {
            *state
                .auto_restore_mode
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            let mode = setting.last_user_mode.unwrap_or(mode);
            let response = tauri_commands::apply_mode_for_lifecycle(mode, false);
            sync_tray_mode(
                app,
                response
                    .control
                    .current_mode
                    .map(schedule::OptimizeMode::from),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::schedule::OptimizeMode;

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
}
