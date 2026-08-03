use crate::backend::{
    admin, autostart, fps, launcher, logging, monitor, process, schedule, settings, shutdown,
    system_info, update,
};
use chrono::{DateTime, Duration as ChronoDuration, Local};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tauri::{ipc::Channel, AppHandle, State};
use windows::Win32::System::Threading::{
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
};

static REAPPLY_GENERATION: AtomicU64 = AtomicU64::new(0);
static LAUNCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static MODE_REQUEST_LOCK: Mutex<()> = Mutex::new(());
static MONITOR_RUNTIME: OnceLock<Mutex<MonitorRuntime>> = OnceLock::new();

struct LauncherClaim<'a>(&'a AtomicBool);

impl<'a> LauncherClaim<'a> {
    fn try_acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for LauncherClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeDto {
    High,
    Normal,
    LowPower,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub current: String,
    pub previous: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlStateDto {
    pub admin_ok: bool,
    pub game_running: bool,
    pub current_mode: Option<ModeDto>,
    pub current_mode_known: bool,
    pub launcher_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateDto {
    pub app_version: String,
    pub status: StatusDto,
    pub control: ControlStateDto,
    pub settings: SettingsStateDto,
    pub update: UpdateStateDto,
    pub monitor: MonitorStateDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResponseDto {
    pub status: StatusDto,
    pub control: ControlStateDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKindDto {
    Daily,
    Weekday,
    Weekend,
    SpecificDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WeekdayDto {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownKindDto {
    Once,
    Weekly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRuleDto {
    pub id: u32,
    pub name: String,
    pub kind: ScheduleKindDto,
    pub date: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub mode: ModeDto,
    pub active: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleStateDto {
    pub active_rule_info: String,
    pub rules: Vec<ScheduleRuleDto>,
    pub empty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCommandResponseDto {
    pub status: StatusDto,
    pub schedule: ScheduleStateDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRuleInputDto {
    pub name: String,
    pub kind: ScheduleKindDto,
    pub date: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub mode: ModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownStateDto {
    pub once_text: String,
    pub once_active: bool,
    pub once_date: Option<String>,
    pub once_time: Option<String>,
    pub weekly_text: String,
    pub weekly_active: bool,
    pub weekly_days: Vec<WeekdayDto>,
    pub weekly_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownCommandResponseDto {
    pub status: StatusDto,
    pub shutdown: ShutdownStateDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownInputDto {
    pub kind: ShutdownKindDto,
    pub date: Option<String>,
    pub time: String,
    pub days: Vec<WeekdayDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeModeDto {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsStateDto {
    pub theme_mode: ThemeModeDto,
    pub effective_dark: bool,
    pub accent_palette: u32,
    pub reduce_motion: bool,
    pub auto_tray_on_game_minimize: bool,
    pub close_to_tray: bool,
    pub autostart_enabled: bool,
    pub autostart_minimized: bool,
    pub launcher_path: String,
    // M96 P3: 게임 감지 시 자동 적용할 기본 모드(None=없음/수동)와 모니터 폴링 간격(ms).
    pub default_mode: Option<ModeDto>,
    pub monitor_interval_ms: u32,
    pub update_alert_enabled: bool,
    pub update_check_interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsCommandResponseDto {
    pub status: StatusDto,
    pub settings: SettingsStateDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKeyDto {
    ThemeMode,
    AccentPalette,
    ReduceMotion,
    AutoTrayOnGameMinimize,
    CloseToTray,
    AutostartEnabled,
    AutostartMinimized,
    LauncherPath,
    DefaultMode,
    MonitorInterval,
    UpdateAlertEnabled,
    UpdateCheckInterval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingInputDto {
    pub key: SettingKeyDto,
    pub theme_mode: Option<ThemeModeDto>,
    pub bool_value: Option<bool>,
    pub string_value: Option<String>,
    // M96 P3: key=DefaultMode 시 사용(null=없음). key=MonitorInterval 시 int_value(ms) 사용.
    pub default_mode: Option<ModeDto>,
    pub int_value: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStateDto {
    pub status_text: String,
    pub available: bool,
    pub checking: bool,
    pub release_url: String,
    pub app_version: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommandResponseDto {
    pub status: StatusDto,
    pub update: UpdateStateDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAlertCommandResponseDto {
    pub status: StatusDto,
    pub update: UpdateStateDto,
    pub should_alert: bool,
    pub alert_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSystemInfoDto {
    pub cpu_name: String,
    pub gpu_name: String,
    pub gpu_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorTotalsDto {
    pub ram_mb: u64,
    pub vram_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorMetricsDto {
    pub cpu_pct: Option<f64>,
    pub mem_mb: Option<u64>,
    pub mem_pct: f64,
    pub gpu_pct: Option<f64>,
    pub vram_mb: Option<u64>,
    pub vram_pct: f64,
    pub disk_read_kbs: Option<u64>,
    pub disk_write_kbs: Option<u64>,
    pub fps: Option<u32>,
    pub fps_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorCoreDto {
    pub index: usize,
    pub usage_pct: f64,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStateDto {
    pub running: bool,
    pub pid: Option<u32>,
    pub system_info: MonitorSystemInfoDto,
    pub totals: MonitorTotalsDto,
    pub metrics: MonitorMetricsDto,
    pub cores: Vec<MonitorCoreDto>,
    pub status_text: String,
}

struct MonitorRuntime {
    monitor: monitor::Monitor,
    fps_pid: Option<u32>,
    fps_starting: bool,
    fps_generation: u64,
    fps_retry: Option<(u32, Instant)>,
    fps_session: Option<fps::FpsSession>,
    system_info: system_info::SystemInfo,
}

impl MonitorRuntime {
    fn new() -> Self {
        Self {
            monitor: monitor::Monitor::new(),
            fps_pid: None,
            fps_starting: false,
            fps_generation: 0,
            fps_retry: None,
            fps_session: None,
            system_info: system_info::fetch_system_info(),
        }
    }
}

impl From<schedule::OptimizeMode> for ModeDto {
    fn from(mode: schedule::OptimizeMode) -> Self {
        match mode {
            schedule::OptimizeMode::High => ModeDto::High,
            schedule::OptimizeMode::Normal => ModeDto::Normal,
            schedule::OptimizeMode::LowPower => ModeDto::LowPower,
        }
    }
}

impl From<ModeDto> for schedule::OptimizeMode {
    fn from(mode: ModeDto) -> Self {
        match mode {
            ModeDto::High => schedule::OptimizeMode::High,
            ModeDto::Normal => schedule::OptimizeMode::Normal,
            ModeDto::LowPower => schedule::OptimizeMode::LowPower,
        }
    }
}

impl From<settings::ThemeMode> for ThemeModeDto {
    fn from(mode: settings::ThemeMode) -> Self {
        match mode {
            settings::ThemeMode::Light => ThemeModeDto::Light,
            settings::ThemeMode::Dark => ThemeModeDto::Dark,
            settings::ThemeMode::System => ThemeModeDto::System,
        }
    }
}

impl From<ThemeModeDto> for settings::ThemeMode {
    fn from(mode: ThemeModeDto) -> Self {
        match mode {
            ThemeModeDto::Light => settings::ThemeMode::Light,
            ThemeModeDto::Dark => settings::ThemeMode::Dark,
            ThemeModeDto::System => settings::ThemeMode::System,
        }
    }
}

fn mode_label(mode: ModeDto) -> &'static str {
    match mode {
        ModeDto::High => "고성능 모드",
        ModeDto::Normal => "일반 모드",
        ModeDto::LowPower => "저전력 모드",
    }
}

fn mode_params(
    mode: schedule::OptimizeMode,
    info: &process::CpuInfo,
) -> (usize, PROCESS_CREATION_FLAGS, &'static str) {
    match mode {
        schedule::OptimizeMode::High => (
            process::calc_high_affinity(info),
            HIGH_PRIORITY_CLASS,
            "고성능 모드 적용 완료.",
        ),
        schedule::OptimizeMode::Normal => (
            process::calc_normal_affinity(info),
            NORMAL_PRIORITY_CLASS,
            "일반 모드 적용 완료.",
        ),
        schedule::OptimizeMode::LowPower => (
            process::calc_low_power_affinity(info),
            IDLE_PRIORITY_CLASS,
            "저전력 모드 적용 완료.",
        ),
    }
}

fn persist_last_user_mode(mode: schedule::OptimizeMode) -> Result<(), settings::SaveError> {
    let _guard = settings::write_lock();
    let mut loaded = settings::load_settings();
    loaded.last_user_mode = Some(mode);
    settings::save_settings(&loaded)
}

fn current_mode_if_running(game_running: bool) -> Option<ModeDto> {
    if !game_running {
        return None;
    }
    process::find_process_id("BlackDesert64.exe")
        .and_then(process::query_current_mode)
        .map(ModeDto::from)
}

fn read_control_state() -> ControlStateDto {
    let loaded = settings::load_settings();
    let game_running = launcher::is_game_running();
    let current_mode = current_mode_if_running(game_running);
    build_control_state(
        admin::is_admin(),
        game_running,
        current_mode,
        loaded.launcher_path,
    )
}

fn build_control_state(
    admin_ok: bool,
    game_running: bool,
    current_mode: Option<ModeDto>,
    launcher_path: String,
) -> ControlStateDto {
    ControlStateDto {
        admin_ok,
        game_running,
        current_mode,
        current_mode_known: current_mode.is_some(),
        launcher_path,
    }
}

#[cfg(test)]
fn control_state_for_test(
    admin_ok: bool,
    game_running: bool,
    current_mode: Option<ModeDto>,
    launcher_path: String,
) -> ControlStateDto {
    build_control_state(admin_ok, game_running, current_mode, launcher_path)
}

fn status(current: impl Into<String>) -> StatusDto {
    StatusDto {
        current: current.into(),
        previous: String::new(),
    }
}

fn command_response(current: impl Into<String>, control: ControlStateDto) -> CommandResponseDto {
    CommandResponseDto {
        status: status(current),
        control,
    }
}

#[cfg(test)]
fn command_response_for_test(current: String, control: ControlStateDto) -> CommandResponseDto {
    command_response(current, control)
}

fn settings_state(
    loaded: settings::AppSettings,
    autostart_enabled: bool,
    autostart_minimized: bool,
) -> SettingsStateDto {
    let update_check_interval_ms =
        if settings::is_supported_update_check_interval_ms(loaded.update_check_interval_ms) {
            loaded.update_check_interval_ms
        } else {
            settings::UPDATE_CHECK_INTERVAL_1D_MS
        };

    SettingsStateDto {
        theme_mode: ThemeModeDto::from(loaded.theme_mode),
        effective_dark: settings::resolve_dark_mode(loaded.theme_mode),
        accent_palette: if settings::is_supported_accent_palette(loaded.accent_palette) {
            loaded.accent_palette
        } else {
            0
        },
        reduce_motion: loaded.reduce_motion,
        auto_tray_on_game_minimize: loaded.auto_tray_on_game_minimize,
        close_to_tray: loaded.close_to_tray,
        autostart_enabled,
        autostart_minimized,
        launcher_path: loaded.launcher_path,
        default_mode: loaded.default_mode.map(ModeDto::from),
        monitor_interval_ms: loaded.monitor_interval_ms,
        update_alert_enabled: loaded.update_alert_enabled,
        update_check_interval_ms,
    }
}

#[cfg(test)]
fn settings_state_for_test() -> SettingsStateDto {
    SettingsStateDto {
        theme_mode: ThemeModeDto::System,
        effective_dark: false,
        accent_palette: 0,
        reduce_motion: false,
        auto_tray_on_game_minimize: false,
        close_to_tray: false,
        autostart_enabled: false,
        autostart_minimized: false,
        launcher_path: String::new(),
        default_mode: None,
        monitor_interval_ms: 1000,
        update_alert_enabled: true,
        update_check_interval_ms: settings::UPDATE_CHECK_INTERVAL_1D_MS,
    }
}

fn read_settings_state() -> SettingsStateDto {
    let loaded = settings::load_settings();
    let (autostart_enabled, autostart_minimized) = autostart::query_autostart();
    settings_state(loaded, autostart_enabled, autostart_minimized)
}

fn settings_command_response(
    current: impl Into<String>,
    settings: SettingsStateDto,
) -> SettingsCommandResponseDto {
    SettingsCommandResponseDto {
        status: status(current),
        settings,
    }
}

fn initial_update_state() -> UpdateStateDto {
    update_state(
        "업데이트 확인 전.".to_string(),
        false,
        false,
        String::new(),
        env!("APP_VERSION").to_string(),
        None,
        None,
    )
}

fn update_state(
    status_text: String,
    available: bool,
    checking: bool,
    release_url: String,
    app_version: String,
    latest_version: Option<String>,
    notes: Option<String>,
) -> UpdateStateDto {
    UpdateStateDto {
        status_text,
        available,
        checking,
        release_url,
        app_version,
        latest_version,
        notes,
    }
}

#[cfg(test)]
fn update_state_for_test(
    status_text: String,
    available: bool,
    checking: bool,
    release_url: String,
    app_version: String,
    latest_version: Option<String>,
    notes: Option<String>,
) -> UpdateStateDto {
    update_state(
        status_text,
        available,
        checking,
        release_url,
        app_version,
        latest_version,
        notes,
    )
}

fn update_state_from_check(check: update::UpdateCheck) -> UpdateStateDto {
    update_state(
        check.status_text,
        check.update_available,
        false,
        String::new(),
        env!("APP_VERSION").to_string(),
        Some(check.latest_version),
        check.notes,
    )
}

fn update_command_response(
    current: impl Into<String>,
    update: UpdateStateDto,
) -> UpdateCommandResponseDto {
    UpdateCommandResponseDto {
        status: status(current),
        update,
    }
}

fn update_alert_command_response(
    current: impl Into<String>,
    update: UpdateStateDto,
    should_alert: bool,
    alert_text: String,
) -> UpdateAlertCommandResponseDto {
    UpdateAlertCommandResponseDto {
        status: status(current),
        update,
        should_alert,
        alert_text,
    }
}

fn should_alert_for_update(
    update_available: bool,
    latest_version: &str,
    last_notified_version: Option<&str>,
) -> bool {
    update_available
        && !latest_version.trim().is_empty()
        && last_notified_version != Some(latest_version)
}

#[cfg(test)]
fn should_alert_for_update_for_test(
    update_available: bool,
    latest_version: &str,
    last_notified_version: Option<&str>,
) -> bool {
    should_alert_for_update(update_available, latest_version, last_notified_version)
}

fn monitor_runtime() -> &'static Mutex<MonitorRuntime> {
    MONITOR_RUNTIME.get_or_init(|| Mutex::new(MonitorRuntime::new()))
}

fn should_claim_fps_start(
    tracked_pid: Option<u32>,
    starting: bool,
    has_session: bool,
    retry_blocked: bool,
    observed_pid: u32,
) -> bool {
    !retry_blocked && (tracked_pid != Some(observed_pid) || (!starting && !has_session))
}

fn fps_start_claim_is_current(
    tracked_pid: Option<u32>,
    starting: bool,
    current_generation: u64,
    claimed_pid: u32,
    claimed_generation: u64,
) -> bool {
    tracked_pid == Some(claimed_pid) && starting && current_generation == claimed_generation
}

fn monitor_system_info(info: &system_info::SystemInfo) -> MonitorSystemInfoDto {
    let gpu_name = if info.gpu_names.is_empty() {
        "Unknown GPU".to_string()
    } else {
        info.gpu_names.join(" / ")
    };
    MonitorSystemInfoDto {
        cpu_name: info.cpu_name.clone(),
        gpu_name,
        gpu_names: info.gpu_names.clone(),
    }
}

#[cfg(test)]
fn monitor_system_info_for_test(cpu_name: String, gpu_names: Vec<String>) -> MonitorSystemInfoDto {
    monitor_system_info(&system_info::SystemInfo {
        cpu_name,
        gpu_names,
    })
}

fn monitor_totals(ram_mb: u64, vram_mb: u64) -> MonitorTotalsDto {
    MonitorTotalsDto { ram_mb, vram_mb }
}

#[cfg(test)]
fn monitor_totals_for_test(ram_mb: u64, vram_mb: u64) -> MonitorTotalsDto {
    monitor_totals(ram_mb, vram_mb)
}

fn clamp_pct(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn pct_of_total(used_mb: Option<u64>, total_mb: u64) -> f64 {
    if total_mb == 0 {
        return 0.0;
    }
    clamp_pct(used_mb.unwrap_or(0) as f64 / total_mb as f64 * 100.0)
}

fn monitor_fps_display(
    current_fps: u32,
    present_events: u64,
    total_events: u64,
    alive: bool,
) -> (Option<u32>, String) {
    let text = if !alive {
        "세션 미시작".to_string()
    } else if present_events > 0 {
        format!("{current_fps} FPS")
    } else if total_events > 0 {
        format!("게임 Present 미수신 ({total_events} ev)")
    } else {
        "ETW 이벤트 없음".to_string()
    };
    let fps = (alive && present_events > 0).then_some(current_fps);
    (fps, text)
}

#[derive(Clone, Copy)]
struct MonitorFpsSnapshot {
    current_fps: u32,
    present_events: u64,
    total_events: u64,
    alive: bool,
}

struct MonitorSampleSnapshot<'a> {
    pid: u32,
    info: &'a system_info::SystemInfo,
    total_ram_mb: u64,
    total_vram_mb: u64,
    sample: &'a monitor::MonitorSample,
    fps: MonitorFpsSnapshot,
}

fn monitor_metrics_from_sample(
    sample: &monitor::MonitorSample,
    total_ram_mb: u64,
    total_vram_mb: u64,
    fps_snapshot: MonitorFpsSnapshot,
) -> MonitorMetricsDto {
    let (fps, fps_text) = monitor_fps_display(
        fps_snapshot.current_fps,
        fps_snapshot.present_events,
        fps_snapshot.total_events,
        fps_snapshot.alive,
    );
    MonitorMetricsDto {
        cpu_pct: sample.cpu_pct.map(clamp_pct),
        mem_mb: sample.mem_mb,
        mem_pct: pct_of_total(sample.mem_mb, total_ram_mb),
        gpu_pct: sample.gpu_pct.map(clamp_pct),
        vram_mb: sample.vram_mb,
        vram_pct: pct_of_total(sample.vram_mb, total_vram_mb),
        disk_read_kbs: sample.disk_read_kbs,
        disk_write_kbs: sample.disk_write_kbs,
        fps,
        fps_text,
    }
}

fn empty_monitor_metrics() -> MonitorMetricsDto {
    MonitorMetricsDto {
        cpu_pct: None,
        mem_mb: None,
        mem_pct: 0.0,
        gpu_pct: None,
        vram_mb: None,
        vram_pct: 0.0,
        disk_read_kbs: None,
        disk_write_kbs: None,
        fps: None,
        fps_text: "세션 미시작".to_string(),
    }
}

fn monitor_cores(sample: &monitor::MonitorSample) -> Vec<MonitorCoreDto> {
    let mask = sample.affinity_mask.unwrap_or(usize::MAX);
    sample
        .core_usages
        .iter()
        .enumerate()
        .map(|(index, usage)| {
            let active = index < usize::BITS as usize && (mask & (1usize << index)) != 0;
            MonitorCoreDto {
                index,
                usage_pct: clamp_pct(*usage),
                active,
            }
        })
        .collect()
}

fn monitor_state(
    running: bool,
    pid: Option<u32>,
    system_info: MonitorSystemInfoDto,
    totals: MonitorTotalsDto,
    metrics: MonitorMetricsDto,
    cores: Vec<MonitorCoreDto>,
    status_text: String,
) -> MonitorStateDto {
    MonitorStateDto {
        running,
        pid,
        system_info,
        totals,
        metrics,
        cores,
        status_text,
    }
}

#[cfg(test)]
fn monitor_state_for_test(
    running: bool,
    pid: Option<u32>,
    system_info: MonitorSystemInfoDto,
    totals: MonitorTotalsDto,
    metrics: MonitorMetricsDto,
    cores: Vec<MonitorCoreDto>,
    status_text: String,
) -> MonitorStateDto {
    monitor_state(
        running,
        pid,
        system_info,
        totals,
        metrics,
        cores,
        status_text,
    )
}

fn monitor_not_running_state(
    info: &system_info::SystemInfo,
    total_ram_mb: u64,
    total_vram_mb: u64,
) -> MonitorStateDto {
    monitor_state(
        false,
        None,
        monitor_system_info(info),
        monitor_totals(total_ram_mb, total_vram_mb),
        empty_monitor_metrics(),
        Vec::new(),
        "BlackDesert64.exe 프로세스를 찾을 수 없습니다.".to_string(),
    )
}

#[cfg(test)]
fn monitor_not_running_state_for_test(
    info: system_info::SystemInfo,
    total_ram_mb: u64,
    total_vram_mb: u64,
) -> MonitorStateDto {
    monitor_not_running_state(&info, total_ram_mb, total_vram_mb)
}

fn monitor_state_from_sample(snapshot: MonitorSampleSnapshot<'_>) -> MonitorStateDto {
    monitor_state(
        true,
        Some(snapshot.pid),
        monitor_system_info(snapshot.info),
        monitor_totals(snapshot.total_ram_mb, snapshot.total_vram_mb),
        monitor_metrics_from_sample(
            snapshot.sample,
            snapshot.total_ram_mb,
            snapshot.total_vram_mb,
            snapshot.fps,
        ),
        monitor_cores(snapshot.sample),
        format!("PID {} 모니터링 중.", snapshot.pid),
    )
}

fn read_initial_monitor_state() -> MonitorStateDto {
    let runtime = monitor_runtime();
    let runtime = runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    monitor_not_running_state(
        &runtime.system_info,
        runtime.monitor.total_ram_mb,
        runtime.monitor.total_vram_mb,
    )
}

fn read_monitor_snapshot() -> MonitorStateDto {
    let observed_pid = process::find_process_id("BlackDesert64.exe");
    let runtime_mutex = monitor_runtime();
    let mut detached_session = None;
    let mut start_claim = None;

    let state = {
        let mut runtime = runtime_mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(pid) = observed_pid {
            let now = Instant::now();
            let retry_blocked = runtime
                .fps_retry
                .is_some_and(|(retry_pid, retry_at)| retry_pid == pid && now < retry_at);
            if should_claim_fps_start(
                runtime.fps_pid,
                runtime.fps_starting,
                runtime.fps_session.is_some(),
                retry_blocked,
                pid,
            ) {
                detached_session = runtime.fps_session.take();
                runtime.fps_pid = Some(pid);
                runtime.fps_starting = true;
                runtime.fps_generation = runtime.fps_generation.wrapping_add(1);
                runtime.fps_retry = None;
                start_claim = Some((pid, runtime.fps_generation, fps::claim_start()));
            }

            let total_ram_mb = runtime.monitor.total_ram_mb;
            let total_vram_mb = runtime.monitor.total_vram_mb;
            let sample = runtime.monitor.sample(pid);
            let (current_fps, present_events, total_events, fps_alive) =
                match runtime.fps_session.as_ref() {
                    Some(session) => (
                        session.current_fps(),
                        session.present_events(),
                        session.total_events(),
                        true,
                    ),
                    None => (0, 0, 0, false),
                };

            monitor_state_from_sample(MonitorSampleSnapshot {
                pid,
                info: &runtime.system_info,
                total_ram_mb,
                total_vram_mb,
                sample: &sample,
                fps: MonitorFpsSnapshot {
                    current_fps,
                    present_events,
                    total_events,
                    alive: fps_alive,
                },
            })
        } else {
            if runtime.fps_pid.is_some() || runtime.fps_starting || runtime.fps_session.is_some() {
                fps::invalidate_start_claims();
            }
            runtime.monitor.rebind(None);
            detached_session = runtime.fps_session.take();
            runtime.fps_pid = None;
            runtime.fps_starting = false;
            runtime.fps_generation = runtime.fps_generation.wrapping_add(1);
            runtime.fps_retry = None;
            monitor_not_running_state(
                &runtime.system_info,
                runtime.monitor.total_ram_mb,
                runtime.monitor.total_vram_mb,
            )
        }
    };

    drop(detached_session);
    if let Some((pid, generation, owner_claim)) = start_claim {
        let started = fps::FpsSession::start(pid, owner_claim).ok();
        let mut stale_session = None;
        {
            let mut runtime = runtime_mutex
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if fps_start_claim_is_current(
                runtime.fps_pid,
                runtime.fps_starting,
                runtime.fps_generation,
                pid,
                generation,
            ) {
                runtime.fps_starting = false;
                if started.is_some() {
                    runtime.fps_session = started;
                    runtime.fps_retry = None;
                } else {
                    runtime.fps_pid = None;
                    runtime.fps_retry = Some((pid, Instant::now() + StdDuration::from_secs(5)));
                }
            } else {
                stale_session = started;
            }
        }
        drop(stale_session);
    }
    state
}

fn validate_setting_input(input: &SettingInputDto) -> Result<(), String> {
    match input.key {
        SettingKeyDto::ThemeMode => input
            .theme_mode
            .map(|_| ())
            .ok_or_else(|| "themeMode 값을 입력하세요.".to_string()),
        SettingKeyDto::AccentPalette => match input.int_value {
            Some(palette) if settings::is_supported_accent_palette(palette) => Ok(()),
            _ => Err("액센트 색상 값이 올바르지 않습니다.".to_string()),
        },
        SettingKeyDto::LauncherPath => input
            .string_value
            .as_deref()
            .map(|_| ())
            .ok_or_else(|| "stringValue 값을 입력하세요.".to_string()),
        SettingKeyDto::ReduceMotion
        | SettingKeyDto::AutoTrayOnGameMinimize
        | SettingKeyDto::CloseToTray
        | SettingKeyDto::AutostartEnabled
        | SettingKeyDto::AutostartMinimized
        | SettingKeyDto::UpdateAlertEnabled => input
            .bool_value
            .map(|_| ())
            .ok_or_else(|| "boolValue 값을 입력하세요.".to_string()),
        // default_mode는 None(없음)도 유효한 선택이므로 추가 검증 없이 통과.
        SettingKeyDto::DefaultMode => Ok(()),
        SettingKeyDto::MonitorInterval => match input.int_value {
            Some(500) | Some(1000) | Some(2000) => Ok(()),
            _ => Err("모니터 갱신 주기는 500/1000/2000(ms)만 허용합니다.".to_string()),
        },
        SettingKeyDto::UpdateCheckInterval => match input.int_value {
            Some(ms) if settings::is_supported_update_check_interval_ms(ms) => Ok(()),
            _ => Err("업데이트 확인 주기는 6시간/12시간/하루/3일/일주일만 허용합니다.".to_string()),
        },
    }
}

#[cfg(test)]
fn validate_setting_input_for_test(input: &SettingInputDto) -> Result<(), String> {
    validate_setting_input(input)
}

fn schedule_kind_to_dto(kind: &schedule::ScheduleKind) -> (ScheduleKindDto, Option<String>) {
    match kind {
        schedule::ScheduleKind::Daily => (ScheduleKindDto::Daily, None),
        schedule::ScheduleKind::Weekday => (ScheduleKindDto::Weekday, None),
        schedule::ScheduleKind::Weekend => (ScheduleKindDto::Weekend, None),
        schedule::ScheduleKind::SpecificDate(date) => {
            (ScheduleKindDto::SpecificDate, Some(date.clone()))
        }
    }
}

fn schedule_rule_dto(rule: &schedule::ScheduleRule) -> ScheduleRuleDto {
    let (kind, date) = schedule_kind_to_dto(&rule.kind);
    ScheduleRuleDto {
        id: rule.id,
        name: rule.name.clone(),
        kind,
        date,
        start_time: rule.start_time.clone(),
        end_time: rule.end_time.clone(),
        mode: ModeDto::from(rule.mode),
        active: rule.active,
        summary: rule.summary(),
    }
}

#[cfg(test)]
fn schedule_rule_dto_for_test(rule: &schedule::ScheduleRule) -> ScheduleRuleDto {
    schedule_rule_dto(rule)
}

fn schedule_state_from_rules(rules: Vec<schedule::ScheduleRule>) -> ScheduleStateDto {
    let active_rule_info = match schedule::active_rule(&rules) {
        Some(rule) => format!("활성 규칙: {}", rule.summary()),
        None => "활성 규칙 없음.".to_string(),
    };
    let empty = rules.is_empty();
    let rules = rules.iter().map(schedule_rule_dto).collect();

    ScheduleStateDto {
        active_rule_info,
        rules,
        empty,
    }
}

fn read_schedule_state() -> ScheduleStateDto {
    schedule_state_from_rules(schedule::load_rules())
}

fn schedule_command_response(
    current: impl Into<String>,
    schedule: ScheduleStateDto,
) -> ScheduleCommandResponseDto {
    ScheduleCommandResponseDto {
        status: status(current),
        schedule,
    }
}

fn schedule_rule_from_input(
    input: ScheduleRuleInputDto,
    id: u32,
) -> Result<schedule::ScheduleRule, String> {
    let name = input.name.trim().to_string();
    let start_time = input.start_time.trim().to_string();
    let end_time = input.end_time.trim().to_string();

    if name.is_empty() || start_time.is_empty() || end_time.is_empty() {
        return Err("규칙 이름, 시작/종료 시간을 모두 입력하세요.".to_string());
    }
    if name.chars().count() > 64 {
        return Err("규칙 이름이 너무 깁니다. 64자 이내로 입력하세요.".to_string());
    }
    if !schedule::validate_time(&start_time) || !schedule::validate_time(&end_time) {
        return Err(
            "시작/종료 시간 형식이 올바르지 않습니다. HH:MM 형식으로 입력하세요.".to_string(),
        );
    }

    let kind = match input.kind {
        ScheduleKindDto::Daily => schedule::ScheduleKind::Daily,
        ScheduleKindDto::Weekday => schedule::ScheduleKind::Weekday,
        ScheduleKindDto::Weekend => schedule::ScheduleKind::Weekend,
        ScheduleKindDto::SpecificDate => {
            let date = input
                .date
                .as_deref()
                .map(str::trim)
                .filter(|date| !date.is_empty())
                .ok_or_else(|| "특정 날짜 규칙에는 날짜를 입력하세요.".to_string())?;
            if !schedule::validate_date(date) {
                return Err(
                    "날짜 형식이 올바르지 않습니다. YYYY-MM-DD 형식으로 입력하세요.".to_string(),
                );
            }
            schedule::ScheduleKind::SpecificDate(date.to_string())
        }
    };

    Ok(schedule::ScheduleRule {
        id,
        name,
        kind,
        start_time,
        end_time,
        mode: schedule::OptimizeMode::from(input.mode),
        active: true,
    })
}

#[cfg(test)]
fn schedule_rule_from_input_for_test(
    input: ScheduleRuleInputDto,
    id: u32,
) -> Result<schedule::ScheduleRule, String> {
    schedule_rule_from_input(input, id)
}

fn fmt_absolute(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

fn fmt_remaining(target: DateTime<Local>, now: DateTime<Local>) -> String {
    fmt_remaining_from_duration(target - now)
}

fn fmt_remaining_from_duration(duration: ChronoDuration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        return "곧 실행".to_string();
    }
    let total_min = secs / 60;
    if total_min < 60 {
        return format!("{total_min}분 남음");
    }
    let total_hours = total_min / 60;
    let mins = total_min % 60;
    if total_hours < 24 {
        if mins == 0 {
            return format!("{total_hours}시간 남음");
        }
        return format!("{total_hours}시간 {mins}분 남음");
    }
    let days = total_hours / 24;
    let hours = total_hours % 24;
    if hours == 0 {
        format!("{days}일 남음")
    } else {
        format!("{days}일 {hours}시간 남음")
    }
}

fn fmt_weekly_days(days: &[&str]) -> String {
    days.iter()
        .map(|day| match *day {
            "MON" => "월",
            "TUE" => "화",
            "WED" => "수",
            "THU" => "목",
            "FRI" => "금",
            "SAT" => "토",
            "SUN" => "일",
            _ => "?",
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn fmt_weekly(info: &shutdown::WeeklyInfo, now: DateTime<Local>) -> String {
    let days_kr = fmt_weekly_days(&info.days);
    let (hour, minute) = info.time_hm;
    format!(
        "매주 {} {:02}:{:02} (다음 {})",
        days_kr,
        hour,
        minute,
        fmt_remaining(info.next_run, now)
    )
}

fn weekday_dto_from_code(day: &str) -> Option<WeekdayDto> {
    match day {
        "MON" => Some(WeekdayDto::Mon),
        "TUE" => Some(WeekdayDto::Tue),
        "WED" => Some(WeekdayDto::Wed),
        "THU" => Some(WeekdayDto::Thu),
        "FRI" => Some(WeekdayDto::Fri),
        "SAT" => Some(WeekdayDto::Sat),
        "SUN" => Some(WeekdayDto::Sun),
        _ => None,
    }
}

fn shutdown_state_from_snapshot(
    snapshot: shutdown::ScheduleSnapshot,
    now: DateTime<Local>,
) -> ShutdownStateDto {
    let (once_text, once_active, once_date, once_time) = match snapshot.once {
        Some(dt) => (
            format!("{} ({})", fmt_absolute(dt), fmt_remaining(dt, now)),
            true,
            Some(dt.format("%Y-%m-%d").to_string()),
            Some(dt.format("%H:%M").to_string()),
        ),
        None => (String::new(), false, None, None),
    };
    let (weekly_text, weekly_active, weekly_days, weekly_time) = match snapshot.weekly {
        Some(info) => {
            let weekly_days = info
                .days
                .iter()
                .filter_map(|day| weekday_dto_from_code(day))
                .collect();
            let (hour, minute) = info.time_hm;
            (
                fmt_weekly(&info, now),
                true,
                weekly_days,
                Some(format!("{hour:02}:{minute:02}")),
            )
        }
        None => (String::new(), false, Vec::new(), None),
    };
    ShutdownStateDto {
        once_text,
        once_active,
        once_date,
        once_time,
        weekly_text,
        weekly_active,
        weekly_days,
        weekly_time,
    }
}

fn read_shutdown_state() -> ShutdownStateDto {
    shutdown_state_from_snapshot(shutdown::query_schedules(), Local::now())
}

fn shutdown_command_response(
    current: impl Into<String>,
    shutdown: ShutdownStateDto,
) -> ShutdownCommandResponseDto {
    ShutdownCommandResponseDto {
        status: status(current),
        shutdown,
    }
}

fn weekday_code(day: WeekdayDto) -> &'static str {
    match day {
        WeekdayDto::Mon => "MON",
        WeekdayDto::Tue => "TUE",
        WeekdayDto::Wed => "WED",
        WeekdayDto::Thu => "THU",
        WeekdayDto::Fri => "FRI",
        WeekdayDto::Sat => "SAT",
        WeekdayDto::Sun => "SUN",
    }
}

fn begin_mode_generation(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::SeqCst) + 1
}

fn generation_is_current(counter: &AtomicU64, generation: u64) -> bool {
    counter.load(Ordering::SeqCst) == generation
}

fn reapply_wait_ms(elapsed_ms: u64, target_ms: u64) -> u64 {
    target_ms.saturating_sub(elapsed_ms)
}

fn schedule_reapply(mode: schedule::OptimizeMode, generation: u64) {
    thread::spawn(move || {
        let started = Instant::now();
        for target_ms in [500_u64, 1000, 2000, 5000, 10000] {
            let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let wait_ms = reapply_wait_ms(elapsed_ms, target_ms);
            if wait_ms > 0 {
                thread::sleep(StdDuration::from_millis(wait_ms));
            }

            let _request_guard = MODE_REQUEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !generation_is_current(&REAPPLY_GENERATION, generation) {
                return;
            }
            let Some(pid) = process::find_process_id("BlackDesert64.exe") else {
                continue;
            };
            let info = process::get_cpu_info();
            let (affinity, priority, _) = mode_params(mode, &info);
            let _ = process::apply_optimization(pid, affinity, priority);
        }
    });
}

fn sync_tray_mode_from_control(app: &tauri::AppHandle, control: &ControlStateDto) {
    crate::tauri_lifecycle::sync_tray_mode(
        app,
        control.current_mode.map(schedule::OptimizeMode::from),
    );
}

#[tauri::command]
pub fn get_app_state(app: tauri::AppHandle) -> AppStateDto {
    let state = AppStateDto {
        app_version: env!("APP_VERSION").to_string(),
        status: status("대기 중입니다."),
        control: read_control_state(),
        settings: read_settings_state(),
        update: initial_update_state(),
        monitor: read_initial_monitor_state(),
    };
    sync_tray_mode_from_control(&app, &state.control);
    state
}

#[tauri::command]
pub async fn get_monitor_snapshot() -> MonitorStateDto {
    tauri::async_runtime::spawn_blocking(read_monitor_snapshot)
        .await
        .unwrap_or_else(|_| read_initial_monitor_state())
}

// 모니터 탭 이탈 시 프런트가 호출한다. ETW FPS 세션을 능동적으로 중단(Drop)해
// 게임 실행 중에도 모니터를 보지 않을 때의 idle ETW 상주 비용을 없앤다.
#[tauri::command]
pub async fn stop_monitor_session() {
    let _ = tauri::async_runtime::spawn_blocking(stop_monitor_session_blocking).await;
}

fn stop_monitor_session_blocking() {
    let runtime = monitor_runtime();
    let detached_session = {
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fps::invalidate_start_claims();
        runtime.fps_pid = None;
        runtime.fps_starting = false;
        runtime.fps_generation = runtime.fps_generation.wrapping_add(1);
        runtime.fps_retry = None;
        runtime.monitor.rebind(None);
        runtime.fps_session.take()
    };
    drop(detached_session);
}

#[tauri::command]
pub fn get_settings() -> SettingsStateDto {
    read_settings_state()
}

fn update_settings(
    mutation: impl FnOnce(&mut settings::AppSettings) -> String,
) -> Result<String, String> {
    let _guard = settings::write_lock();
    let mut loaded = settings::load_settings();
    let message = mutation(&mut loaded);
    settings::save_settings(&loaded).map_err(|error| error.to_string())?;
    Ok(message)
}

#[tauri::command]
pub fn set_setting(input: SettingInputDto) -> Result<SettingsCommandResponseDto, String> {
    if let Err(message) = validate_setting_input(&input) {
        return Ok(settings_command_response(message, read_settings_state()));
    }

    let message = match input.key {
        SettingKeyDto::ThemeMode => update_settings(|loaded| {
            loaded.theme_mode = settings::ThemeMode::from(input.theme_mode.unwrap());
            "테마 설정을 저장했습니다.".to_string()
        }),
        SettingKeyDto::AccentPalette => update_settings(|loaded| {
            loaded.accent_palette = input.int_value.unwrap();
            "액센트 색상을 저장했습니다.".to_string()
        }),
        SettingKeyDto::ReduceMotion => update_settings(|loaded| {
            loaded.reduce_motion = input.bool_value.unwrap();
            "접근성 설정을 저장했습니다.".to_string()
        }),
        SettingKeyDto::AutoTrayOnGameMinimize => update_settings(|loaded| {
            loaded.auto_tray_on_game_minimize = input.bool_value.unwrap();
            "런처 동작 설정을 저장했습니다.".to_string()
        }),
        SettingKeyDto::CloseToTray => update_settings(|loaded| {
            loaded.close_to_tray = input.bool_value.unwrap();
            "창 닫기 동작 설정을 저장했습니다.".to_string()
        }),
        SettingKeyDto::LauncherPath => {
            let path = input.string_value.unwrap();
            if path.chars().count() > 512 {
                Ok("런처 경로가 너무 깁니다. 512자 이내로 입력하세요.".to_string())
            } else {
                update_settings(move |loaded| {
                    loaded.launcher_path = path;
                    "런처 경로를 저장했습니다.".to_string()
                })
            }
        }
        SettingKeyDto::DefaultMode => update_settings(|loaded| {
            loaded.default_mode = input.default_mode.map(schedule::OptimizeMode::from);
            match loaded.default_mode {
                Some(_) => "기본 적용 모드를 저장했습니다.".to_string(),
                None => "기본 적용 모드를 사용 안 함으로 설정했습니다.".to_string(),
            }
        }),
        SettingKeyDto::MonitorInterval => update_settings(|loaded| {
            loaded.monitor_interval_ms = input.int_value.unwrap();
            "모니터 갱신 주기를 저장했습니다.".to_string()
        }),
        SettingKeyDto::UpdateAlertEnabled => update_settings(|loaded| {
            loaded.update_alert_enabled = input.bool_value.unwrap();
            if loaded.update_alert_enabled {
                "업데이트 알림을 켰습니다.".to_string()
            } else {
                "업데이트 알림을 껐습니다.".to_string()
            }
        }),
        SettingKeyDto::UpdateCheckInterval => update_settings(|loaded| {
            loaded.update_check_interval_ms = input.int_value.unwrap();
            "업데이트 확인 주기를 저장했습니다.".to_string()
        }),
        SettingKeyDto::AutostartEnabled => {
            let on = input.bool_value.unwrap();
            let result = if on {
                let (_, minimized) = autostart::query_autostart();
                autostart::register_autostart(minimized)
            } else {
                autostart::unregister_autostart()
            };
            match result {
                Ok(()) if on => Ok("자동 시작을 등록했습니다.".to_string()),
                Ok(()) => Ok("자동 시작을 해제했습니다.".to_string()),
                Err(autostart::Error::TaskNotFound) if !on => {
                    Ok("자동 시작이 이미 해제되어 있습니다.".to_string())
                }
                Err(error) => Err(error.to_string()),
            }
        }
        SettingKeyDto::AutostartMinimized => {
            let on = input.bool_value.unwrap();
            let (enabled, _) = autostart::query_autostart();
            if !enabled {
                Ok("자동 시작이 꺼져 있습니다.".to_string())
            } else {
                match autostart::register_autostart(on) {
                    Ok(()) if on => Ok("자동 시작을 트레이 시작으로 변경했습니다.".to_string()),
                    Ok(()) => Ok("자동 시작을 일반 창으로 변경했습니다.".to_string()),
                    Err(error) => Err(error.to_string()),
                }
            }
        }
    }?;

    Ok(settings_command_response(message, read_settings_state()))
}

#[tauri::command]
pub fn open_log_folder() -> StatusDto {
    match logging::open_log_folder() {
        Ok(()) => status("로그 폴더를 열었습니다."),
        Err(error) => status(error.to_string()),
    }
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    pending_update: State<'_, update::PendingUpdateState>,
) -> Result<UpdateCommandResponseDto, String> {
    Ok(
        match update::check_latest_release(&app, &pending_update).await {
            Ok(check) => {
                let state = update_state_from_check(check);
                update_command_response(state.status_text.clone(), state)
            }
            Err(error) => {
                let message = error.to_string();
                update_command_response(
                    message.clone(),
                    update_state(
                        message,
                        false,
                        false,
                        String::new(),
                        env!("APP_VERSION").to_string(),
                        None,
                        None,
                    ),
                )
            }
        },
    )
}

#[tauri::command]
pub async fn check_update_alert(
    app: AppHandle,
    pending_update: State<'_, update::PendingUpdateState>,
) -> Result<UpdateAlertCommandResponseDto, String> {
    if !settings::load_settings().update_alert_enabled {
        return Ok(update_alert_command_response(
            "업데이트 알림이 꺼져 있습니다.",
            initial_update_state(),
            false,
            String::new(),
        ));
    }

    match update::check_latest_release(&app, &pending_update).await {
        Ok(check) => {
            let should_alert = if check.update_available {
                let _guard = settings::write_lock();
                let mut loaded = settings::load_settings();
                if loaded.update_alert_enabled
                    && should_alert_for_update(
                        true,
                        &check.latest_version,
                        loaded.last_update_notified_version.as_deref(),
                    )
                {
                    loaded.last_update_notified_version = Some(check.latest_version.clone());
                    settings::save_settings(&loaded).map_err(|error| error.to_string())?;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let state = update_state_from_check(check);
            let alert_text = if should_alert {
                state.status_text.clone()
            } else {
                String::new()
            };
            Ok(update_alert_command_response(
                state.status_text.clone(),
                state,
                should_alert,
                alert_text,
            ))
        }
        Err(error) => {
            let message = error.to_string();
            Ok(update_alert_command_response(
                message.clone(),
                update_state(
                    message,
                    false,
                    false,
                    String::new(),
                    env!("APP_VERSION").to_string(),
                    None,
                    None,
                ),
                false,
                String::new(),
            ))
        }
    }
}

#[tauri::command]
pub async fn install_update(
    pending_update: State<'_, update::PendingUpdateState>,
    on_event: Channel<update::UpdateProgressEvent>,
) -> Result<StatusDto, String> {
    update::install_pending_update(&pending_update, on_event)
        .await
        .map(|()| status("업데이트 설치 프로그램을 시작했습니다."))
        .map_err(|error| error.to_string())
}

// M96: 앱 푸터에서 여는 GitHub 저장소 URL. open_release_page의 github.com 화이트리스트를 통과한다.
const REPOSITORY_URL: &str = "https://github.com/Lv2dev/bdo-optimizer-launcher";

#[tauri::command]
pub fn open_repository() -> StatusDto {
    match update::open_release_page(REPOSITORY_URL) {
        Ok(()) => status("GitHub 저장소를 열었습니다."),
        Err(error) => status(error.to_string()),
    }
}

#[tauri::command]
pub fn refresh_game_status(app: tauri::AppHandle) -> CommandResponseDto {
    let control = read_control_state();
    let message = if control.game_running {
        "게임 실행 중 (BlackDesert64.exe 확인됨)."
    } else {
        "게임 미실행 상태."
    };
    let response = command_response(message, control);
    sync_tray_mode_from_control(&app, &response.control);
    response
}

#[tauri::command]
pub async fn launch_game(launcher_path: String) -> CommandResponseDto {
    if launcher_path.chars().count() > 512 {
        return command_response(
            "런처 경로가 너무 깁니다. 512자 이내로 입력하세요.",
            read_control_state(),
        );
    }

    let result = match run_claimed_blocking(&LAUNCH_IN_PROGRESS, move || {
        launcher::launch_game(&launcher_path)
    })
    .await
    {
        Ok(Some(result)) => Ok(result),
        Ok(None) => {
            return command_response(
                "런처 실행을 준비 중입니다. 잠시만 기다려 주세요.",
                read_control_state(),
            )
        }
        Err(error) => Err(error),
    };
    let message = match result {
        Ok(launcher::LaunchResult::GameAlreadyRunning) => {
            "게임이 이미 실행 중입니다. 런처를 실행하지 않습니다.".to_string()
        }
        Ok(launcher::LaunchResult::LauncherStarted(path)) => {
            format!("런처 실행됨: {}", path.display())
        }
        Ok(launcher::LaunchResult::LauncherRejected(path, error)) => {
            format!("런처 보안 검증 실패 ({}): {error}", path.display())
        }
        Ok(launcher::LaunchResult::LauncherNotFound) => {
            "런처를 찾을 수 없습니다. 경로를 직접 입력하세요.".to_string()
        }
        Err(error) => format!("런처 실행 작업 실패: {error}"),
    };

    command_response(message, read_control_state())
}

async fn run_claimed_blocking<T, F>(
    flag: &'static AtomicBool,
    operation: F,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_blocking(move || {
        let _claim = LauncherClaim::try_acquire(flag)?;
        Some(operation())
    })
    .await
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PreparedModeRequest {
    ApplyNow { generation: u64 },
    DeferredBySchedule(schedule::OptimizeMode),
}

fn prepare_mode_request<E>(
    requested: schedule::OptimizeMode,
    persist_user_choice: bool,
    active_schedule: Option<schedule::OptimizeMode>,
    save: impl FnOnce(schedule::OptimizeMode) -> Result<(), E>,
    begin_generation: impl FnOnce() -> u64,
) -> Result<PreparedModeRequest, E> {
    if persist_user_choice {
        if let Some(active_mode) = active_schedule {
            save(requested)?;
            return Ok(PreparedModeRequest::DeferredBySchedule(active_mode));
        }
    }
    Ok(PreparedModeRequest::ApplyNow {
        generation: begin_generation(),
    })
}

fn run_serialized_mode_decision<T, R>(
    lock: &Mutex<()>,
    decide: impl FnOnce() -> Option<T>,
    apply: impl FnOnce(T) -> R,
) -> Option<R> {
    let _request_guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    decide().map(apply)
}

fn apply_mode_impl_locked(
    backend_mode: schedule::OptimizeMode,
    persist_user_choice: bool,
) -> CommandResponseDto {
    let active_schedule = persist_user_choice
        .then(|| schedule::active_rule(&schedule::load_rules()).map(|rule| rule.mode))
        .flatten();
    let prepared = prepare_mode_request(
        backend_mode,
        persist_user_choice,
        active_schedule,
        persist_last_user_mode,
        || begin_mode_generation(&REAPPLY_GENERATION),
    );
    let generation = match prepared {
        Ok(PreparedModeRequest::ApplyNow { generation }) => generation,
        Ok(PreparedModeRequest::DeferredBySchedule(_)) => {
            return command_response(
                "수동 모드를 저장했습니다. 활성 스케줄이 끝날 때까지 스케줄 모드를 유지합니다.",
                read_control_state(),
            );
        }
        Err(error) => {
            return command_response(
                format!("수동 모드 저장에 실패했습니다: {error}"),
                read_control_state(),
            );
        }
    };
    let info = process::get_cpu_info();
    let mode = ModeDto::from(backend_mode);
    let (affinity, priority, success_text) = mode_params(backend_mode, &info);

    let Some(pid) = process::find_process_id_fresh("BlackDesert64.exe") else {
        return command_response(
            "BlackDesert64.exe 프로세스를 찾을 수 없습니다.",
            read_control_state(),
        );
    };

    let message = match process::apply_optimization(pid, affinity, priority) {
        Ok(()) => {
            tracing::info!(
                pid,
                mode = mode_label(mode),
                affinity = format_args!("{:#x}", affinity),
                hybrid = info.has_hybrid,
                "mode applied from tauri command"
            );
            let persistence_error = persist_user_choice
                .then(|| persist_last_user_mode(backend_mode).err())
                .flatten();
            schedule_reapply(backend_mode, generation);
            match persistence_error {
                Some(error) => format!("모드는 적용했지만 설정 저장에 실패했습니다: {error}"),
                None => success_text.to_string(),
            }
        }
        Err(e) => {
            tracing::error!(pid, mode = mode_label(mode), error = %e, "tauri mode apply failed");
            format!("오류: {e}")
        }
    };

    command_response(message, read_control_state())
}

fn apply_mode_impl(
    backend_mode: schedule::OptimizeMode,
    persist_user_choice: bool,
) -> CommandResponseDto {
    let _request_guard = MODE_REQUEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    apply_mode_impl_locked(backend_mode, persist_user_choice)
}

pub(crate) fn apply_mode_for_lifecycle(
    mode: schedule::OptimizeMode,
    persist_user_choice: bool,
) -> CommandResponseDto {
    apply_mode_impl(mode, persist_user_choice)
}

pub(crate) fn apply_mode_for_lifecycle_decision(
    decide: impl FnOnce() -> Option<schedule::OptimizeMode>,
) -> Option<CommandResponseDto> {
    run_serialized_mode_decision(&MODE_REQUEST_LOCK, decide, |mode| {
        apply_mode_impl_locked(mode, false)
    })
}

#[tauri::command]
pub fn apply_mode(app: tauri::AppHandle, mode: ModeDto) -> CommandResponseDto {
    let response = apply_mode_impl(schedule::OptimizeMode::from(mode), true);
    sync_tray_mode_from_control(&app, &response.control);
    response
}

#[tauri::command]
pub fn list_schedule_rules() -> ScheduleStateDto {
    read_schedule_state()
}

fn next_schedule_id(rules: &[schedule::ScheduleRule]) -> Result<u32, String> {
    schedule::next_id(rules).map_err(str::to_string)
}

#[tauri::command]
pub fn add_schedule_rule(
    input: ScheduleRuleInputDto,
) -> Result<ScheduleCommandResponseDto, String> {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let id = next_schedule_id(&rules)?;
    let message = match schedule_rule_from_input(input, id) {
        Ok(rule) => {
            rules.push(rule);
            schedule::save_rules(&rules).map_err(|error| error.to_string())?;
            "스케줄 규칙이 추가되었습니다.".to_string()
        }
        Err(message) => message,
    };
    Ok(schedule_command_response(
        message,
        schedule_state_from_rules(rules),
    ))
}

#[tauri::command]
pub fn delete_schedule_rule(id: u32) -> Result<ScheduleCommandResponseDto, String> {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let before = rules.len();
    rules.retain(|rule| rule.id != id);
    let message = if rules.len() == before {
        "삭제할 스케줄 규칙을 찾을 수 없습니다.".to_string()
    } else {
        schedule::save_rules(&rules).map_err(|error| error.to_string())?;
        "스케줄 규칙이 삭제되었습니다.".to_string()
    };
    Ok(schedule_command_response(
        message,
        schedule_state_from_rules(rules),
    ))
}

#[tauri::command]
pub fn toggle_schedule_rule(id: u32) -> Result<ScheduleCommandResponseDto, String> {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let message = match rules.iter_mut().find(|rule| rule.id == id) {
        Some(rule) => {
            rule.active = !rule.active;
            schedule::save_rules(&rules).map_err(|error| error.to_string())?;
            "스케줄 규칙 상태가 변경되었습니다.".to_string()
        }
        None => "변경할 스케줄 규칙을 찾을 수 없습니다.".to_string(),
    };
    Ok(schedule_command_response(
        message,
        schedule_state_from_rules(rules),
    ))
}

#[tauri::command]
pub fn get_shutdown_state() -> ShutdownStateDto {
    read_shutdown_state()
}

#[tauri::command]
pub fn register_shutdown(input: ShutdownInputDto) -> ShutdownCommandResponseDto {
    let time = input.time.trim().to_string();
    let message = match input.kind {
        ShutdownKindDto::Once => {
            let date = input
                .date
                .as_deref()
                .map(str::trim)
                .filter(|date| !date.is_empty())
                .map(str::to_string);
            match date {
                Some(date) => match shutdown::register_once_shutdown(&date, &time) {
                    Ok(()) => format!("단발 종료 예약 등록 완료: {date} {time}."),
                    Err(error) => format!("오류: {error}"),
                },
                None => "날짜를 입력하세요.".to_string(),
            }
        }
        ShutdownKindDto::Weekly => {
            let days = input
                .days
                .iter()
                .copied()
                .map(weekday_code)
                .collect::<Vec<_>>();
            match shutdown::register_weekly_shutdown(&days, &time) {
                Ok(()) => format!("매주 반복 종료 예약 등록 완료 (매주 {time})."),
                Err(error) => format!("오류: {error}"),
            }
        }
    };
    shutdown_command_response(message, read_shutdown_state())
}

#[tauri::command]
pub fn cancel_shutdown(kind: ShutdownKindDto) -> ShutdownCommandResponseDto {
    let message = match kind {
        ShutdownKindDto::Once => match shutdown::cancel_once() {
            Ok(()) => "단발 종료 예약이 취소되었습니다.".to_string(),
            Err(error) => format!("오류: {error}"),
        },
        ShutdownKindDto::Weekly => match shutdown::cancel_weekly() {
            Ok(()) => "매주 반복 종료 예약이 취소되었습니다.".to_string(),
            Err(error) => format!("오류: {error}"),
        },
    };
    shutdown_command_response(message, read_shutdown_state())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn mode_dto_serializes_as_tauri_wire_tokens() {
        assert_eq!(serde_json::to_string(&ModeDto::High).unwrap(), "\"high\"");
        assert_eq!(
            serde_json::to_string(&ModeDto::Normal).unwrap(),
            "\"normal\""
        );
        assert_eq!(
            serde_json::to_string(&ModeDto::LowPower).unwrap(),
            "\"low_power\""
        );
    }

    #[test]
    fn control_state_uses_camel_case_wire_shape() {
        let state = control_state_for_test(
            true,
            false,
            Some(ModeDto::LowPower),
            r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe".to_string(),
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "adminOk": true,
                "gameRunning": false,
                "currentMode": "low_power",
                "currentModeKnown": true,
                "launcherPath": r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe"
            })
        );
    }

    #[test]
    fn command_response_carries_status_and_control_state() {
        let response = command_response_for_test(
            "고성능 모드 적용 완료.".to_string(),
            control_state_for_test(true, true, Some(ModeDto::High), String::new()),
        );

        let value = serde_json::to_value(response).unwrap();

        assert_eq!(value["status"]["current"], "고성능 모드 적용 완료.");
        assert_eq!(value["status"]["previous"], "");
        assert_eq!(value["control"]["currentMode"], "high");
    }

    #[test]
    fn settings_state_uses_camel_case_wire_shape() {
        let mut state = settings_state_for_test();
        state.effective_dark = true;
        state.auto_tray_on_game_minimize = true;
        state.close_to_tray = true;
        state.autostart_enabled = true;
        state.accent_palette = 2;
        state.launcher_path = r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe".to_string();

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "themeMode": "system",
                "effectiveDark": true,
                "accentPalette": 2,
                "reduceMotion": false,
                "autoTrayOnGameMinimize": true,
                "closeToTray": true,
                "autostartEnabled": true,
                "autostartMinimized": false,
                "launcherPath": r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe",
                "defaultMode": null,
                "monitorIntervalMs": 1000,
                "updateAlertEnabled": true,
                "updateCheckIntervalMs": 86400000
            })
        );
    }

    #[test]
    fn setting_input_uses_expected_wire_shape() {
        let input = SettingInputDto {
            key: SettingKeyDto::ThemeMode,
            theme_mode: Some(ThemeModeDto::Dark),
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: None,
        };

        let value = serde_json::to_value(input).unwrap();

        assert_eq!(
            value,
            json!({
                "key": "theme_mode",
                "themeMode": "dark",
                "boolValue": null,
                "stringValue": null,
                "defaultMode": null,
                "intValue": null
            })
        );
    }

    #[test]
    fn setting_input_rejects_missing_bool_value() {
        let input = SettingInputDto {
            key: SettingKeyDto::ReduceMotion,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: None,
        };

        let err = validate_setting_input_for_test(&input).unwrap_err();

        assert!(err.contains("boolValue"));
    }

    #[test]
    fn monitor_interval_accepts_only_supported_values() {
        let make = |ms: Option<u32>| SettingInputDto {
            key: SettingKeyDto::MonitorInterval,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: ms,
        };

        assert!(validate_setting_input_for_test(&make(Some(500))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(1000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(2000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(1500))).is_err());
        assert!(validate_setting_input_for_test(&make(None)).is_err());
    }

    #[test]
    fn update_check_interval_accepts_only_supported_values() {
        let make = |ms: Option<u32>| SettingInputDto {
            key: SettingKeyDto::UpdateCheckInterval,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: ms,
        };

        assert!(validate_setting_input_for_test(&make(Some(21_600_000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(43_200_000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(86_400_000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(259_200_000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(604_800_000))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(3_600_000))).is_err());
        assert!(validate_setting_input_for_test(&make(None)).is_err());
    }

    #[test]
    fn accent_palette_accepts_only_supported_values() {
        let make = |palette: Option<u32>| SettingInputDto {
            key: SettingKeyDto::AccentPalette,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: palette,
        };

        assert!(validate_setting_input_for_test(&make(Some(0))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(1))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(2))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(3))).is_ok());
        assert!(validate_setting_input_for_test(&make(Some(4))).is_err());
        assert!(validate_setting_input_for_test(&make(None)).is_err());
    }

    #[test]
    fn update_alert_setting_requires_bool_value() {
        let input = SettingInputDto {
            key: SettingKeyDto::UpdateAlertEnabled,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: None,
        };

        let err = validate_setting_input_for_test(&input).unwrap_err();

        assert!(err.contains("boolValue"));
    }

    #[test]
    fn update_alert_notification_decision_dedupes_versions() {
        assert!(should_alert_for_update_for_test(true, "0.1.2", None));
        assert!(should_alert_for_update_for_test(
            true,
            "0.1.2",
            Some("0.1.1")
        ));
        assert!(!should_alert_for_update_for_test(
            true,
            "0.1.2",
            Some("0.1.2")
        ));
        assert!(!should_alert_for_update_for_test(false, "0.1.2", None));
    }

    #[test]
    fn default_mode_setting_accepts_none_selection() {
        let input = SettingInputDto {
            key: SettingKeyDto::DefaultMode,
            theme_mode: None,
            bool_value: None,
            string_value: None,
            default_mode: None,
            int_value: None,
        };

        assert!(validate_setting_input_for_test(&input).is_ok());
    }

    #[test]
    fn update_state_uses_camel_case_wire_shape() {
        let state = update_state_for_test(
            "새 버전 0.2.0 사용 가능.".to_string(),
            true,
            false,
            "https://github.com/owner/repo/releases/tag/v0.2.0".to_string(),
            "0.1.0".to_string(),
            Some("0.2.0".to_string()),
            Some("변경 사항".to_string()),
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "statusText": "새 버전 0.2.0 사용 가능.",
                "available": true,
                "checking": false,
                "releaseUrl": "https://github.com/owner/repo/releases/tag/v0.2.0",
                "appVersion": "0.1.0",
                "latestVersion": "0.2.0",
                "notes": "변경 사항"
            })
        );
    }

    #[test]
    fn monitor_state_uses_camel_case_wire_shape() {
        let state = monitor_state_for_test(
            true,
            Some(4321),
            monitor_system_info_for_test(
                "AMD Ryzen 7 7800X3D".to_string(),
                vec!["NVIDIA GeForce RTX 4080".to_string()],
            ),
            monitor_totals_for_test(32768, 16384),
            MonitorMetricsDto {
                cpu_pct: Some(31.5),
                mem_mb: Some(8192),
                mem_pct: 25.0,
                gpu_pct: Some(55.0),
                vram_mb: Some(4096),
                vram_pct: 25.0,
                disk_read_kbs: Some(120),
                disk_write_kbs: Some(48),
                fps: Some(144),
                fps_text: "144 FPS".to_string(),
            },
            vec![
                MonitorCoreDto {
                    index: 0,
                    usage_pct: 12.5,
                    active: true,
                },
                MonitorCoreDto {
                    index: 1,
                    usage_pct: 88.0,
                    active: false,
                },
            ],
            "PID 4321 모니터링 중.".to_string(),
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "running": true,
                "pid": 4321,
                "systemInfo": {
                    "cpuName": "AMD Ryzen 7 7800X3D",
                    "gpuName": "NVIDIA GeForce RTX 4080",
                    "gpuNames": ["NVIDIA GeForce RTX 4080"]
                },
                "totals": {
                    "ramMb": 32768,
                    "vramMb": 16384
                },
                "metrics": {
                    "cpuPct": 31.5,
                    "memMb": 8192,
                    "memPct": 25.0,
                    "gpuPct": 55.0,
                    "vramMb": 4096,
                    "vramPct": 25.0,
                    "diskReadKbs": 120,
                    "diskWriteKbs": 48,
                    "fps": 144,
                    "fpsText": "144 FPS"
                },
                "cores": [
                    { "index": 0, "usagePct": 12.5, "active": true },
                    { "index": 1, "usagePct": 88.0, "active": false }
                ],
                "statusText": "PID 4321 모니터링 중."
            })
        );
    }

    #[test]
    fn fps_session_start_is_claimed_once_per_observed_pid() {
        assert!(should_claim_fps_start(None, false, false, false, 700));
        assert!(should_claim_fps_start(Some(699), false, true, false, 700));
        assert!(!should_claim_fps_start(Some(700), true, false, false, 700));
        assert!(!should_claim_fps_start(Some(700), false, true, false, 700));
        assert!(should_claim_fps_start(Some(700), false, false, false, 700));
        assert!(!should_claim_fps_start(None, false, false, true, 700));
        assert!(fps_start_claim_is_current(Some(700), true, 4, 700, 4));
        assert!(!fps_start_claim_is_current(Some(700), true, 5, 700, 4));
    }

    #[test]
    fn fps_display_distinguishes_foreign_events_and_expired_game_fps() {
        assert_eq!(
            monitor_fps_display(0, 0, 10, true),
            (None, "게임 Present 미수신 (10 ev)".to_string())
        );
        assert_eq!(
            monitor_fps_display(0, 3, 10, true),
            (Some(0), "0 FPS".to_string())
        );
    }

    #[test]
    fn monitor_sample_converts_percentages_and_core_affinity() {
        let sample = crate::backend::monitor::MonitorSample {
            cpu_pct: Some(31.5),
            mem_mb: Some(8192),
            gpu_pct: Some(55.0),
            vram_mb: Some(4096),
            disk_read_kbs: Some(120),
            disk_write_kbs: Some(48),
            core_usages: vec![12.5, 88.0],
            affinity_mask: Some(0b01),
        };

        let info = crate::backend::system_info::SystemInfo {
            cpu_name: "AMD Ryzen 7 7800X3D".to_string(),
            gpu_names: vec!["NVIDIA GeForce RTX 4080".to_string()],
        };
        let state = monitor_state_from_sample(MonitorSampleSnapshot {
            pid: 4321,
            info: &info,
            total_ram_mb: 32768,
            total_vram_mb: 8192,
            sample: &sample,
            fps: MonitorFpsSnapshot {
                current_fps: 144,
                present_events: 8,
                total_events: 8,
                alive: true,
            },
        });

        assert_eq!(state.metrics.mem_pct, 25.0);
        assert_eq!(state.metrics.vram_pct, 50.0);
        assert_eq!(state.metrics.fps, Some(144));
        assert_eq!(state.metrics.fps_text, "144 FPS");
        assert!(state.cores[0].active);
        assert!(!state.cores[1].active);
    }

    #[test]
    fn monitor_not_running_state_preserves_system_info() {
        let state = monitor_not_running_state_for_test(
            crate::backend::system_info::SystemInfo {
                cpu_name: "Intel CPU".to_string(),
                gpu_names: Vec::new(),
            },
            0,
            0,
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(value["running"], false);
        assert_eq!(value["pid"], serde_json::Value::Null);
        assert_eq!(value["systemInfo"]["cpuName"], "Intel CPU");
        assert_eq!(value["systemInfo"]["gpuName"], "Unknown GPU");
        assert_eq!(value["metrics"]["fpsText"], "세션 미시작");
        assert_eq!(
            value["statusText"],
            "BlackDesert64.exe 프로세스를 찾을 수 없습니다."
        );
    }

    #[test]
    fn app_state_carries_settings_update_and_monitor_state() {
        let mut settings = settings_state_for_test();
        settings.theme_mode = ThemeModeDto::Dark;
        settings.effective_dark = true;
        settings.reduce_motion = true;
        settings.close_to_tray = true;

        let state = AppStateDto {
            app_version: "0.1.0".to_string(),
            status: status("대기 중입니다."),
            control: control_state_for_test(false, false, None, String::new()),
            settings,
            update: update_state_for_test(
                "업데이트 채널 미설정.".to_string(),
                false,
                false,
                String::new(),
                "0.1.0".to_string(),
                None,
                None,
            ),
            monitor: monitor_not_running_state_for_test(
                crate::backend::system_info::SystemInfo {
                    cpu_name: "Intel CPU".to_string(),
                    gpu_names: vec!["GPU".to_string()],
                },
                16384,
                8192,
            ),
        };

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(value["settings"]["themeMode"], "dark");
        assert_eq!(value["settings"]["reduceMotion"], true);
        assert_eq!(value["update"]["statusText"], "업데이트 채널 미설정.");
        assert_eq!(value["update"]["available"], false);
        assert_eq!(value["monitor"]["totals"]["ramMb"], 16384);
        assert_eq!(value["monitor"]["systemInfo"]["gpuName"], "GPU");
    }

    #[test]
    fn initial_update_state_uses_default_release_channel() {
        let state = initial_update_state();

        assert_eq!(state.status_text, "업데이트 확인 전.");
        assert!(!state.available);
        assert!(!state.checking);
        assert_eq!(state.release_url, "");
    }

    #[test]
    fn schedule_rule_dto_uses_tauri_wire_shape() {
        let rule = schedule::ScheduleRule {
            id: 7,
            name: "야간 저전력".to_string(),
            kind: schedule::ScheduleKind::SpecificDate("2026-06-03".to_string()),
            start_time: "22:00".to_string(),
            end_time: "06:00".to_string(),
            mode: schedule::OptimizeMode::LowPower,
            active: true,
        };

        let value = serde_json::to_value(schedule_rule_dto_for_test(&rule)).unwrap();

        assert_eq!(
            value,
            json!({
                "id": 7,
                "name": "야간 저전력",
                "kind": "specific_date",
                "date": "2026-06-03",
                "startTime": "22:00",
                "endTime": "06:00",
                "mode": "low_power",
                "active": true,
                "summary": "야간 저전력 | 2026-06-03 | 22:00-06:00 | 저전력"
            })
        );
    }

    #[test]
    fn schedule_input_rejects_specific_date_without_date() {
        let input = ScheduleRuleInputDto {
            name: "특정일".to_string(),
            kind: ScheduleKindDto::SpecificDate,
            date: None,
            start_time: "19:00".to_string(),
            end_time: "23:00".to_string(),
            mode: ModeDto::High,
        };

        let err = schedule_rule_from_input_for_test(input, 1).unwrap_err();

        assert!(err.contains("날짜"));
    }

    #[test]
    fn shutdown_state_uses_camel_case_wire_shape() {
        let state = ShutdownStateDto {
            once_text: "2026-06-03 23:30 (1시간 남음)".to_string(),
            once_active: true,
            once_date: Some("2026-06-03".to_string()),
            once_time: Some("23:30".to_string()),
            weekly_text: "매주 월/수 05:00 (다음 2일 남음)".to_string(),
            weekly_active: true,
            weekly_days: vec![WeekdayDto::Mon, WeekdayDto::Wed],
            weekly_time: Some("05:00".to_string()),
        };

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "onceText": "2026-06-03 23:30 (1시간 남음)",
                "onceActive": true,
                "onceDate": "2026-06-03",
                "onceTime": "23:30",
                "weeklyText": "매주 월/수 05:00 (다음 2일 남음)",
                "weeklyActive": true,
                "weeklyDays": ["MON", "WED"],
                "weeklyTime": "05:00"
            })
        );
    }

    #[test]
    fn shutdown_state_from_weekly_snapshot_carries_form_values() {
        let now = Local
            .with_ymd_and_hms(2026, 6, 4, 10, 0, 0)
            .single()
            .unwrap();
        let next_run = Local
            .with_ymd_and_hms(2026, 6, 7, 5, 7, 0)
            .single()
            .unwrap();
        let state = shutdown_state_from_snapshot(
            shutdown::ScheduleSnapshot {
                once: None,
                weekly: Some(shutdown::WeeklyInfo {
                    days: vec!["MON", "SUN"],
                    time_hm: (5, 7),
                    next_run,
                }),
            },
            now,
        );

        assert!(state.weekly_active);
        assert_eq!(state.weekly_days, vec![WeekdayDto::Mon, WeekdayDto::Sun]);
        assert_eq!(state.weekly_time, Some("05:07".to_string()));
    }

    #[test]
    fn shutdown_state_from_once_snapshot_carries_form_values() {
        let now = Local
            .with_ymd_and_hms(2026, 7, 11, 10, 0, 0)
            .single()
            .unwrap();
        let once = Local
            .with_ymd_and_hms(2026, 7, 12, 23, 30, 0)
            .single()
            .unwrap();
        let state = shutdown_state_from_snapshot(
            shutdown::ScheduleSnapshot {
                once: Some(once),
                weekly: None,
            },
            now,
        );

        assert_eq!(state.once_date, Some("2026-07-12".to_string()));
        assert_eq!(state.once_time, Some("23:30".to_string()));
    }

    #[test]
    fn weekly_shutdown_input_serializes_days_as_scheduler_tokens() {
        let input = ShutdownInputDto {
            kind: ShutdownKindDto::Weekly,
            date: None,
            time: "05:00".to_string(),
            days: vec![WeekdayDto::Mon, WeekdayDto::Sun],
        };

        let value = serde_json::to_value(input).unwrap();

        assert_eq!(
            value,
            json!({
                "kind": "weekly",
                "date": null,
                "time": "05:00",
                "days": ["MON", "SUN"]
            })
        );
    }

    #[test]
    fn new_mode_request_invalidates_previous_generation_even_if_it_later_fails() {
        let counter = AtomicU64::new(0);
        let first = begin_mode_generation(&counter);
        let failed_request = begin_mode_generation(&counter);

        assert!(!generation_is_current(&counter, first));
        assert!(generation_is_current(&counter, failed_request));
    }

    #[test]
    fn reapply_waits_are_based_on_request_start_not_previous_sleep() {
        assert_eq!(500, reapply_wait_ms(0, 500));
        assert_eq!(500, reapply_wait_ms(500, 1000));
        assert_eq!(1000, reapply_wait_ms(1000, 2000));
        assert_eq!(3000, reapply_wait_ms(2000, 5000));
        assert_eq!(5000, reapply_wait_ms(5000, 10000));
        assert_eq!(0, reapply_wait_ms(11000, 10000));
    }

    #[test]
    fn active_schedule_persists_manual_choice_without_immediate_os_apply() {
        let save_calls = std::cell::Cell::new(0);
        let generation_calls = std::cell::Cell::new(0);
        let scheduled = prepare_mode_request::<()>(
            schedule::OptimizeMode::Normal,
            true,
            Some(schedule::OptimizeMode::High),
            |mode| {
                assert_eq!(mode, schedule::OptimizeMode::Normal);
                save_calls.set(save_calls.get() + 1);
                Ok(())
            },
            || {
                generation_calls.set(generation_calls.get() + 1);
                1
            },
        )
        .unwrap();
        assert_eq!(
            scheduled,
            PreparedModeRequest::DeferredBySchedule(schedule::OptimizeMode::High)
        );
        assert_eq!(save_calls.get(), 1);
        assert_eq!(generation_calls.get(), 0);

        let inactive = prepare_mode_request::<()>(
            schedule::OptimizeMode::LowPower,
            true,
            None,
            |_| panic!("inactive schedule must persist only after a successful apply"),
            || 9,
        )
        .unwrap();
        assert_eq!(inactive, PreparedModeRequest::ApplyNow { generation: 9 });
    }

    #[test]
    fn active_schedule_save_failure_does_not_start_a_generation() {
        let generation_calls = std::cell::Cell::new(0);
        let result = prepare_mode_request(
            schedule::OptimizeMode::Normal,
            true,
            Some(schedule::OptimizeMode::High),
            |_| Err("save failed"),
            || {
                generation_calls.set(generation_calls.get() + 1);
                1
            },
        );

        assert_eq!(result, Err("save failed"));
        assert_eq!(generation_calls.get(), 0);
    }

    #[test]
    fn lifecycle_mode_decision_and_apply_are_atomic_against_user_requests() {
        let lock = std::sync::Arc::new(Mutex::new(()));
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let (decision_started_tx, decision_started_rx) = std::sync::mpsc::channel();
        let (release_decision_tx, release_decision_rx) = std::sync::mpsc::channel();

        let lifecycle_lock = std::sync::Arc::clone(&lock);
        let decision_lock = std::sync::Arc::clone(&lock);
        let apply_lock = std::sync::Arc::clone(&lock);
        let lifecycle_events = std::sync::Arc::clone(&events);
        let lifecycle = std::thread::spawn(move || {
            run_serialized_mode_decision(
                &lifecycle_lock,
                || {
                    assert!(matches!(
                        decision_lock.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    lifecycle_events
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push("lifecycle_decision");
                    decision_started_tx.send(()).unwrap();
                    release_decision_rx.recv().unwrap();
                    Some(schedule::OptimizeMode::High)
                },
                |mode| {
                    assert!(matches!(
                        apply_lock.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    lifecycle_events
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push("lifecycle_apply");
                    mode
                },
            )
        });

        decision_started_rx.recv().unwrap();
        let user_lock = std::sync::Arc::clone(&lock);
        let user_events = std::sync::Arc::clone(&events);
        let (user_attempted_tx, user_attempted_rx) = std::sync::mpsc::channel();
        let user = std::thread::spawn(move || {
            user_attempted_tx.send(()).unwrap();
            let _guard = user_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            user_events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("user_apply");
        });

        user_attempted_rx.recv().unwrap();
        release_decision_tx.send(()).unwrap();
        assert_eq!(
            lifecycle.join().unwrap(),
            Some(schedule::OptimizeMode::High)
        );
        user.join().unwrap();

        assert_eq!(
            *events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["lifecycle_decision", "lifecycle_apply", "user_apply"]
        );
    }

    #[test]
    fn blocking_launcher_work_runs_off_the_command_thread() {
        let command_thread = std::thread::current().id();
        let worker_thread =
            tauri::async_runtime::block_on(run_blocking(|| std::thread::current().id())).unwrap();

        assert_ne!(command_thread, worker_thread);
    }

    #[test]
    fn blocking_launcher_panic_is_returned_as_an_error() {
        let result = tauri::async_runtime::block_on(run_blocking(|| -> () {
            panic!("injected launcher panic")
        }));

        assert!(result.is_err());
    }

    #[test]
    fn launcher_single_flight_claim_releases_on_drop() {
        let flag = AtomicBool::new(false);
        let first = LauncherClaim::try_acquire(&flag).unwrap();
        assert!(LauncherClaim::try_acquire(&flag).is_none());
        drop(first);
        assert!(LauncherClaim::try_acquire(&flag).is_some());
    }

    #[test]
    fn add_schedule_id_overflow_is_returned_as_a_command_error() {
        let rules = vec![schedule::ScheduleRule {
            id: u32::MAX,
            name: "last".to_string(),
            kind: schedule::ScheduleKind::Daily,
            start_time: "09:00".to_string(),
            end_time: "10:00".to_string(),
            mode: schedule::OptimizeMode::Normal,
            active: true,
        }];

        assert_eq!(
            next_schedule_id(&rules),
            Err("스케줄 규칙 ID 공간이 소진되었습니다.".to_string())
        );
    }

    #[test]
    fn cancelled_launcher_future_keeps_claim_until_blocking_work_finishes() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        FLAG.store(false, Ordering::Release);

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let task = tauri::async_runtime::spawn(run_claimed_blocking(&FLAG, move || {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
        }));

        started_rx
            .recv_timeout(StdDuration::from_secs(2))
            .expect("blocking launcher work did not start");
        task.abort();
        let _ = tauri::async_runtime::block_on(task);
        assert!(FLAG.load(Ordering::Acquire));

        finish_tx.send(()).unwrap();
        let deadline = Instant::now() + StdDuration::from_secs(2);
        while FLAG.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!FLAG.load(Ordering::Acquire));
    }
}
