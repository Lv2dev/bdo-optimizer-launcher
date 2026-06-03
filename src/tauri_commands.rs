use crate::backend::{
    admin, autostart, fps, launcher, logging, monitor, process, schedule, settings, shutdown,
    system_info, update,
};
use chrono::{DateTime, Duration as ChronoDuration, Local};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration as StdDuration;
use windows::Win32::System::Threading::{
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
};

static REAPPLY_GENERATION: AtomicU64 = AtomicU64::new(0);
static MONITOR_RUNTIME: OnceLock<Mutex<MonitorRuntime>> = OnceLock::new();

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
    pub weekly_text: String,
    pub weekly_active: bool,
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
    pub reduce_motion: bool,
    pub auto_tray_on_game_minimize: bool,
    pub close_to_tray: bool,
    pub autostart_enabled: bool,
    pub autostart_minimized: bool,
    pub launcher_path: String,
    // M96 P3: 게임 감지 시 자동 적용할 기본 모드(None=없음/수동)와 모니터 폴링 간격(ms).
    pub default_mode: Option<ModeDto>,
    pub monitor_interval_ms: u32,
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
    ReduceMotion,
    AutoTrayOnGameMinimize,
    CloseToTray,
    AutostartEnabled,
    AutostartMinimized,
    LauncherPath,
    DefaultMode,
    MonitorInterval,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommandResponseDto {
    pub status: StatusDto,
    pub update: UpdateStateDto,
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
    fps_session: Option<fps::FpsSession>,
    system_info: system_info::SystemInfo,
}

impl MonitorRuntime {
    fn new() -> Self {
        Self {
            monitor: monitor::Monitor::new(),
            fps_pid: None,
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

fn persist_last_user_mode(mode: schedule::OptimizeMode) {
    let _guard = settings::write_lock();
    let mut loaded = settings::load_settings();
    loaded.last_user_mode = Some(mode);
    settings::save_settings(&loaded);
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
    SettingsStateDto {
        theme_mode: ThemeModeDto::from(loaded.theme_mode),
        effective_dark: settings::resolve_dark_mode(loaded.theme_mode),
        reduce_motion: loaded.reduce_motion,
        auto_tray_on_game_minimize: loaded.auto_tray_on_game_minimize,
        close_to_tray: loaded.close_to_tray,
        autostart_enabled,
        autostart_minimized,
        launcher_path: loaded.launcher_path,
        default_mode: loaded.default_mode.map(ModeDto::from),
        monitor_interval_ms: loaded.monitor_interval_ms,
    }
}

#[cfg(test)]
fn settings_state_for_test() -> SettingsStateDto {
    SettingsStateDto {
        theme_mode: ThemeModeDto::System,
        effective_dark: false,
        reduce_motion: false,
        auto_tray_on_game_minimize: false,
        close_to_tray: false,
        autostart_enabled: false,
        autostart_minimized: false,
        launcher_path: String::new(),
        default_mode: None,
        monitor_interval_ms: 1000,
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
    let status_text = if update::configured_release_api_url().is_some() {
        "업데이트 확인 전."
    } else {
        "업데이트 채널 미설정."
    };
    update_state(
        status_text.to_string(),
        false,
        false,
        String::new(),
        env!("APP_VERSION").to_string(),
    )
}

fn update_state(
    status_text: String,
    available: bool,
    checking: bool,
    release_url: String,
    app_version: String,
) -> UpdateStateDto {
    UpdateStateDto {
        status_text,
        available,
        checking,
        release_url,
        app_version,
    }
}

#[cfg(test)]
fn update_state_for_test(
    status_text: String,
    available: bool,
    checking: bool,
    release_url: String,
    app_version: String,
) -> UpdateStateDto {
    update_state(status_text, available, checking, release_url, app_version)
}

fn update_state_from_check(check: update::UpdateCheck) -> UpdateStateDto {
    update_state(
        check.status_text,
        check.update_available,
        false,
        check.release_url,
        env!("APP_VERSION").to_string(),
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

fn monitor_runtime() -> &'static Mutex<MonitorRuntime> {
    MONITOR_RUNTIME.get_or_init(|| Mutex::new(MonitorRuntime::new()))
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
    } else if current_fps > 0 {
        format!("{current_fps} FPS")
    } else if present_events > 0 {
        "측정 중...".to_string()
    } else if total_events > 0 {
        format!("Present 미수신 ({total_events} ev)")
    } else {
        "ETW 이벤트 없음".to_string()
    };
    let fps = alive.then_some(current_fps);
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
    let runtime = monitor_runtime();
    let mut runtime = runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let Some(pid) = process::find_process_id("BlackDesert64.exe") else {
        runtime.monitor.rebind(None);
        runtime.fps_pid = None;
        runtime.fps_session = None;
        return monitor_not_running_state(
            &runtime.system_info,
            runtime.monitor.total_ram_mb,
            runtime.monitor.total_vram_mb,
        );
    };

    if runtime.fps_pid != Some(pid) {
        runtime.fps_session = fps::FpsSession::start(pid).ok();
        runtime.fps_pid = Some(pid);
    }

    let total_ram_mb = runtime.monitor.total_ram_mb;
    let total_vram_mb = runtime.monitor.total_vram_mb;
    let sample = runtime.monitor.sample(pid);
    let (current_fps, present_events, total_events, fps_alive) = match runtime.fps_session.as_ref()
    {
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
}

fn validate_setting_input(input: &SettingInputDto) -> Result<(), String> {
    match input.key {
        SettingKeyDto::ThemeMode => input
            .theme_mode
            .map(|_| ())
            .ok_or_else(|| "themeMode 값을 입력하세요.".to_string()),
        SettingKeyDto::LauncherPath => input
            .string_value
            .as_deref()
            .map(|_| ())
            .ok_or_else(|| "stringValue 값을 입력하세요.".to_string()),
        SettingKeyDto::ReduceMotion
        | SettingKeyDto::AutoTrayOnGameMinimize
        | SettingKeyDto::CloseToTray
        | SettingKeyDto::AutostartEnabled
        | SettingKeyDto::AutostartMinimized => input
            .bool_value
            .map(|_| ())
            .ok_or_else(|| "boolValue 값을 입력하세요.".to_string()),
        // default_mode는 None(없음)도 유효한 선택이므로 추가 검증 없이 통과.
        SettingKeyDto::DefaultMode => Ok(()),
        SettingKeyDto::MonitorInterval => match input.int_value {
            Some(500) | Some(1000) | Some(2000) => Ok(()),
            _ => Err("모니터 갱신 주기는 500/1000/2000(ms)만 허용합니다.".to_string()),
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

fn shutdown_state(
    once_text: String,
    once_active: bool,
    weekly_text: String,
    weekly_active: bool,
) -> ShutdownStateDto {
    ShutdownStateDto {
        once_text,
        once_active,
        weekly_text,
        weekly_active,
    }
}

#[cfg(test)]
fn shutdown_state_for_test(
    once_text: String,
    once_active: bool,
    weekly_text: String,
    weekly_active: bool,
) -> ShutdownStateDto {
    shutdown_state(once_text, once_active, weekly_text, weekly_active)
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

fn shutdown_state_from_snapshot(
    snapshot: shutdown::ScheduleSnapshot,
    now: DateTime<Local>,
) -> ShutdownStateDto {
    let (once_text, once_active) = match snapshot.once {
        Some(dt) => (
            format!("{} ({})", fmt_absolute(dt), fmt_remaining(dt, now)),
            true,
        ),
        None => (String::new(), false),
    };
    let (weekly_text, weekly_active) = match snapshot.weekly {
        Some(info) => (fmt_weekly(&info, now), true),
        None => (String::new(), false),
    };
    shutdown_state(once_text, once_active, weekly_text, weekly_active)
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

fn schedule_reapply(mode: schedule::OptimizeMode) {
    let generation = REAPPLY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    thread::spawn(move || {
        for ms in [500_u64, 1000, 2000, 5000, 10000] {
            thread::sleep(StdDuration::from_millis(ms));
            if REAPPLY_GENERATION.load(Ordering::SeqCst) != generation {
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
pub fn get_monitor_snapshot() -> MonitorStateDto {
    read_monitor_snapshot()
}

// 모니터 탭 이탈 시 프런트가 호출한다. ETW FPS 세션을 능동적으로 중단(Drop)해
// 게임 실행 중에도 모니터를 보지 않을 때의 idle ETW 상주 비용을 없앤다.
#[tauri::command]
pub fn stop_monitor_session() {
    let runtime = monitor_runtime();
    let mut runtime = runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    runtime.fps_session = None;
    runtime.fps_pid = None;
    runtime.monitor.rebind(None);
}

#[tauri::command]
pub fn get_settings() -> SettingsStateDto {
    read_settings_state()
}

#[tauri::command]
pub fn set_setting(input: SettingInputDto) -> SettingsCommandResponseDto {
    if let Err(message) = validate_setting_input(&input) {
        return settings_command_response(message, read_settings_state());
    }

    // load→수정→save를 직렬화해 동시 호출 시 갱신 유실을 막는다.
    let _guard = settings::write_lock();
    let message = match input.key {
        SettingKeyDto::ThemeMode => {
            let mut loaded = settings::load_settings();
            loaded.theme_mode = settings::ThemeMode::from(input.theme_mode.unwrap());
            settings::save_settings(&loaded);
            "테마 설정을 저장했습니다.".to_string()
        }
        SettingKeyDto::ReduceMotion => {
            let mut loaded = settings::load_settings();
            loaded.reduce_motion = input.bool_value.unwrap();
            settings::save_settings(&loaded);
            "접근성 설정을 저장했습니다.".to_string()
        }
        SettingKeyDto::AutoTrayOnGameMinimize => {
            let mut loaded = settings::load_settings();
            loaded.auto_tray_on_game_minimize = input.bool_value.unwrap();
            settings::save_settings(&loaded);
            "런처 동작 설정을 저장했습니다.".to_string()
        }
        SettingKeyDto::CloseToTray => {
            let mut loaded = settings::load_settings();
            loaded.close_to_tray = input.bool_value.unwrap();
            settings::save_settings(&loaded);
            "창 닫기 동작 설정을 저장했습니다.".to_string()
        }
        SettingKeyDto::LauncherPath => {
            let path = input.string_value.unwrap();
            if path.chars().count() > 512 {
                "런처 경로가 너무 깁니다. 512자 이내로 입력하세요.".to_string()
            } else {
                let mut loaded = settings::load_settings();
                loaded.launcher_path = path;
                settings::save_settings(&loaded);
                "런처 경로를 저장했습니다.".to_string()
            }
        }
        SettingKeyDto::DefaultMode => {
            let mut loaded = settings::load_settings();
            loaded.default_mode = input.default_mode.map(schedule::OptimizeMode::from);
            settings::save_settings(&loaded);
            match loaded.default_mode {
                Some(_) => "기본 적용 모드를 저장했습니다.".to_string(),
                None => "기본 적용 모드를 사용 안 함으로 설정했습니다.".to_string(),
            }
        }
        SettingKeyDto::MonitorInterval => {
            let mut loaded = settings::load_settings();
            loaded.monitor_interval_ms = input.int_value.unwrap();
            settings::save_settings(&loaded);
            "모니터 갱신 주기를 저장했습니다.".to_string()
        }
        SettingKeyDto::AutostartEnabled => {
            let on = input.bool_value.unwrap();
            let result = if on {
                let (_, minimized) = autostart::query_autostart();
                autostart::register_autostart(minimized)
            } else {
                autostart::unregister_autostart()
            };
            match result {
                Ok(()) if on => "자동 시작을 등록했습니다.".to_string(),
                Ok(()) => "자동 시작을 해제했습니다.".to_string(),
                Err(autostart::Error::TaskNotFound) if !on => {
                    "자동 시작이 이미 해제되어 있습니다.".to_string()
                }
                Err(error) => error.to_string(),
            }
        }
        SettingKeyDto::AutostartMinimized => {
            let on = input.bool_value.unwrap();
            let (enabled, _) = autostart::query_autostart();
            if !enabled {
                "자동 시작이 꺼져 있습니다.".to_string()
            } else {
                match autostart::register_autostart(on) {
                    Ok(()) if on => "자동 시작을 트레이 시작으로 변경했습니다.".to_string(),
                    Ok(()) => "자동 시작을 일반 창으로 변경했습니다.".to_string(),
                    Err(error) => error.to_string(),
                }
            }
        }
    };

    settings_command_response(message, read_settings_state())
}

#[tauri::command]
pub fn open_log_folder() -> StatusDto {
    match logging::open_log_folder() {
        Ok(()) => status("로그 폴더를 열었습니다."),
        Err(error) => status(error.to_string()),
    }
}

#[tauri::command]
pub fn check_for_updates() -> UpdateCommandResponseDto {
    match update::check_latest_release() {
        Ok(check) => {
            let state = update_state_from_check(check);
            update_command_response(state.status_text.clone(), state)
        }
        Err(update::Error::ChannelNotConfigured) => update_command_response(
            "업데이트 채널이 설정되지 않았습니다.",
            update_state(
                "업데이트 채널 미설정.".to_string(),
                false,
                false,
                String::new(),
                env!("APP_VERSION").to_string(),
            ),
        ),
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
                ),
            )
        }
    }
}

#[tauri::command]
pub fn open_update_release(url: String) -> StatusDto {
    if url.trim().is_empty() {
        return status("열 수 있는 릴리스 페이지가 없습니다.");
    }
    match update::open_release_page(&url) {
        Ok(()) => status("GitHub Release 페이지를 열었습니다."),
        Err(error) => status(error.to_string()),
    }
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
pub fn launch_game(launcher_path: String) -> CommandResponseDto {
    if launcher_path.chars().count() > 512 {
        return command_response(
            "런처 경로가 너무 깁니다. 512자 이내로 입력하세요.",
            read_control_state(),
        );
    }

    let message = match launcher::launch_game(&launcher_path) {
        launcher::LaunchResult::GameAlreadyRunning => {
            "게임이 이미 실행 중입니다. 런처를 실행하지 않습니다.".to_string()
        }
        launcher::LaunchResult::LauncherStarted(path) => format!("런처 실행됨: {}", path.display()),
        launcher::LaunchResult::LauncherNotFound => {
            "런처를 찾을 수 없습니다. 경로를 직접 입력하세요.".to_string()
        }
    };

    command_response(message, read_control_state())
}

fn apply_mode_impl(
    backend_mode: schedule::OptimizeMode,
    persist_user_choice: bool,
) -> CommandResponseDto {
    let info = process::get_cpu_info();
    let mode = ModeDto::from(backend_mode);
    let (affinity, priority, success_text) = mode_params(backend_mode, &info);

    let Some(pid) = process::find_process_id("BlackDesert64.exe") else {
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
            if persist_user_choice {
                persist_last_user_mode(backend_mode);
            }
            schedule_reapply(backend_mode);
            success_text.to_string()
        }
        Err(e) => {
            tracing::error!(pid, mode = mode_label(mode), error = %e, "tauri mode apply failed");
            format!("오류: {e}")
        }
    };

    command_response(message, read_control_state())
}

pub(crate) fn apply_mode_for_lifecycle(
    mode: schedule::OptimizeMode,
    persist_user_choice: bool,
) -> CommandResponseDto {
    apply_mode_impl(mode, persist_user_choice)
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

#[tauri::command]
pub fn add_schedule_rule(input: ScheduleRuleInputDto) -> ScheduleCommandResponseDto {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let id = schedule::next_id(&rules);
    let message = match schedule_rule_from_input(input, id) {
        Ok(rule) => {
            rules.push(rule);
            schedule::save_rules(&rules);
            "스케줄 규칙이 추가되었습니다.".to_string()
        }
        Err(message) => message,
    };
    schedule_command_response(message, schedule_state_from_rules(rules))
}

#[tauri::command]
pub fn delete_schedule_rule(id: u32) -> ScheduleCommandResponseDto {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let before = rules.len();
    rules.retain(|rule| rule.id != id);
    let message = if rules.len() == before {
        "삭제할 스케줄 규칙을 찾을 수 없습니다.".to_string()
    } else {
        schedule::save_rules(&rules);
        "스케줄 규칙이 삭제되었습니다.".to_string()
    };
    schedule_command_response(message, schedule_state_from_rules(rules))
}

#[tauri::command]
pub fn toggle_schedule_rule(id: u32) -> ScheduleCommandResponseDto {
    let _guard = schedule::write_lock();
    let mut rules = schedule::load_rules();
    let message = match rules.iter_mut().find(|rule| rule.id == id) {
        Some(rule) => {
            rule.active = !rule.active;
            schedule::save_rules(&rules);
            "스케줄 규칙 상태가 변경되었습니다.".to_string()
        }
        None => "변경할 스케줄 규칙을 찾을 수 없습니다.".to_string(),
    };
    schedule_command_response(message, schedule_state_from_rules(rules))
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
                Ok(()) => format!("매주 반복 종료 예약 등록 완료 ({time}시)."),
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
        state.launcher_path = r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe".to_string();

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "themeMode": "system",
                "effectiveDark": true,
                "reduceMotion": false,
                "autoTrayOnGameMinimize": true,
                "closeToTray": true,
                "autostartEnabled": true,
                "autostartMinimized": false,
                "launcherPath": r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe",
                "defaultMode": null,
                "monitorIntervalMs": 1000
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
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "statusText": "새 버전 0.2.0 사용 가능.",
                "available": true,
                "checking": false,
                "releaseUrl": "https://github.com/owner/repo/releases/tag/v0.2.0",
                "appVersion": "0.1.0"
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
        let state = shutdown_state_for_test(
            "2026-06-03 23:30 (1시간 남음)".to_string(),
            true,
            "매주 월/수 05:00 (다음 2일 남음)".to_string(),
            true,
        );

        let value = serde_json::to_value(state).unwrap();

        assert_eq!(
            value,
            json!({
                "onceText": "2026-06-03 23:30 (1시간 남음)",
                "onceActive": true,
                "weeklyText": "매주 월/수 05:00 (다음 2일 남음)",
                "weeklyActive": true
            })
        );
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
}
