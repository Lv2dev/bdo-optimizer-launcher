export const EMPTY_STATE = {
  appVersion: "0.2.0",
  status: { current: "초기화 중입니다.", previous: "" },
  control: {
    adminOk: false,
    gameRunning: false,
    currentMode: null,
    currentModeKnown: false,
    launcherPath: "",
  },
  schedule: { activeRuleInfo: "활성 규칙 없음.", rules: [], empty: true },
  shutdown: {
    onceText: "",
    onceActive: false,
    onceDate: null,
    onceTime: null,
    weeklyText: "",
    weeklyActive: false,
    weeklyDays: [],
    weeklyTime: null,
  },
  settings: {
    themeMode: "system",
    effectiveDark: true,
    accentPalette: 0,
    reduceMotion: false,
    autoTrayOnGameMinimize: false,
    closeToTray: true,
    autostartEnabled: false,
    autostartMinimized: false,
    launcherPath: "",
    defaultMode: null,
    monitorIntervalMs: 1000,
    updateAlertEnabled: true,
    updateCheckIntervalMs: 86400000,
  },
  update: {
    statusText: "업데이트 확인 전.",
    available: false,
    checking: false,
    releaseUrl: "",
    appVersion: "0.2.0",
    latestVersion: null,
  },
  monitor: {
    running: false,
    pid: null,
    systemInfo: { cpuName: "Unknown CPU", gpuName: "Unknown GPU", gpuNames: [] },
    totals: { ramMb: 0, vramMb: 0 },
    metrics: {
      cpuPct: null,
      memMb: null,
      memPct: 0,
      gpuPct: null,
      vramMb: null,
      vramPct: 0,
      diskReadKbs: null,
      diskWriteKbs: null,
      fps: null,
      fpsText: "세션 미시작",
    },
    cores: [],
    statusText: "BlackDesert64.exe 프로세스를 찾을 수 없습니다.",
  },
};

const MODE_LABELS = { high: "고성능", normal: "일반", low_power: "저전력" };
const KIND_LABELS = { daily: "매일", weekday: "평일", weekend: "주말" };
const WEEKDAY_LABELS = { MON: "월", TUE: "화", WED: "수", THU: "목", FRI: "금", SAT: "토", SUN: "일" };
const UPDATE_INTERVALS = new Set([21600000, 43200000, 86400000, 259200000, 604800000]);

const status = (current) => ({ current, previous: "" });

function scheduleSummary(input) {
  const kind = input.kind === "specific_date" ? input.date : KIND_LABELS[input.kind];
  return `${input.name} | ${kind} | ${input.startTime}-${input.endTime} | ${MODE_LABELS[input.mode]}`;
}

export function createBrowserPreview() {
  let rules = [];
  let shutdown = { ...EMPTY_STATE.shutdown };
  let settings = { ...EMPTY_STATE.settings };
  let update = { ...EMPTY_STATE.update };
  let monitor = { ...EMPTY_STATE.monitor };
  let monitorTick = 0;
  const nextRuleId = () => rules.reduce((maximum, rule) => Math.max(maximum, rule.id), 0) + 1;

  const scheduleState = () => {
    const active = rules.find((rule) => rule.active);
    return {
      activeRuleInfo: active ? `활성 규칙: ${active.summary}` : "활성 규칙 없음.",
      rules,
      empty: rules.length === 0,
    };
  };

  const monitorState = () => {
    monitorTick += 1;
    const wave = (base, amplitude, phase = 0) =>
      Math.max(0, Math.min(100, base + Math.sin(monitorTick / 3 + phase) * amplitude));
    const memMb = Math.round(8200 + Math.sin(monitorTick / 5) * 260);
    const vramMb = Math.round(4100 + Math.sin(monitorTick / 4 + 0.4) * 180);
    monitor = {
      running: true,
      pid: 4321,
      systemInfo: {
        cpuName: "AMD Ryzen 7 7800X3D 8-Core Processor",
        gpuName: "NVIDIA GeForce RTX 4080",
        gpuNames: ["NVIDIA GeForce RTX 4080"],
      },
      totals: { ramMb: 32768, vramMb: 12288 },
      metrics: {
        cpuPct: wave(18, 9),
        memMb,
        memPct: (memMb / 32768) * 100,
        gpuPct: wave(34, 16, 1.2),
        vramMb,
        vramPct: (vramMb / 12288) * 100,
        diskReadKbs: Math.round(120 + wave(60, 45, 2)),
        diskWriteKbs: Math.round(36 + wave(18, 12, 3)),
        fps: 144,
        fpsText: "144 FPS",
      },
      cores: Array.from({ length: 8 }, (_, index) => ({
        index,
        usagePct: wave(16 + index * 2, 18, index / 2),
        active: true,
      })),
      statusText: "브라우저 미리보기 모니터링 중.",
    };
    return monitor;
  };

  return (command, args = {}) => {
    if (command === "apply_mode") {
      return {
        status: status(`${MODE_LABELS[args.mode]} 모드 미리보기.`),
        control: { ...EMPTY_STATE.control, currentMode: args.mode, currentModeKnown: true },
      };
    }
    if (command === "launch_game") {
      return {
        status: status("Tauri 앱에서 게임 실행 명령을 사용할 수 있습니다."),
        control: EMPTY_STATE.control,
      };
    }
    if (command === "refresh_game_status") {
      return { status: status("브라우저 미리보기 상태입니다."), control: EMPTY_STATE.control };
    }
    if (command === "list_schedule_rules") return scheduleState();
    if (command === "add_schedule_rule") {
      const input = args.input;
      const normalized = {
        ...input,
        name: input.name.trim() || "이름 없는 규칙",
        date: input.kind === "specific_date" ? input.date : null,
      };
      rules = [
        ...rules,
        { id: nextRuleId(), ...normalized, active: true, summary: scheduleSummary(normalized) },
      ];
      return { status: status("스케줄 규칙이 추가되었습니다."), schedule: scheduleState() };
    }
    if (command === "delete_schedule_rule") {
      rules = rules.filter((rule) => rule.id !== args.id);
      return { status: status("스케줄 규칙이 삭제되었습니다."), schedule: scheduleState() };
    }
    if (command === "toggle_schedule_rule") {
      rules = rules.map((rule) =>
        rule.id === args.id ? { ...rule, active: !rule.active } : rule,
      );
      return { status: status("스케줄 규칙 상태가 변경되었습니다."), schedule: scheduleState() };
    }
    if (command === "get_shutdown_state") return shutdown;
    if (command === "get_settings") return settings;
    if (command === "get_monitor_snapshot") return monitorState();
    if (command === "stop_monitor_session") return null;
    if (command === "set_setting") {
      const input = args.input;
      const scalarFields = {
        launcher_path: ["launcherPath", input.stringValue ?? ""],
        default_mode: ["defaultMode", input.defaultMode ?? null],
        monitor_interval: ["monitorIntervalMs", input.intValue ?? 1000],
        accent_palette: ["accentPalette", Number.isInteger(input.intValue) && input.intValue >= 0 && input.intValue < 4 ? input.intValue : 0],
        update_check_interval: ["updateCheckIntervalMs", UPDATE_INTERVALS.has(Number(input.intValue)) ? Number(input.intValue) : 86400000],
      };
      if (input.key === "theme_mode") {
        const themeMode = input.themeMode ?? settings.themeMode;
        settings = { ...settings, themeMode, effectiveDark: themeMode !== "light" };
      } else if (scalarFields[input.key]) {
        const [field, value] = scalarFields[input.key];
        settings = { ...settings, [field]: value };
      } else {
        const booleanFields = {
          reduce_motion: "reduceMotion",
          auto_tray_on_game_minimize: "autoTrayOnGameMinimize",
          close_to_tray: "closeToTray",
          autostart_enabled: "autostartEnabled",
          autostart_minimized: "autostartMinimized",
          update_alert_enabled: "updateAlertEnabled",
        };
        const field = booleanFields[input.key];
        if (field) settings = { ...settings, [field]: Boolean(input.boolValue) };
        if (input.key === "autostart_enabled" && !input.boolValue) {
          settings = { ...settings, autostartMinimized: false };
        }
      }
      return { status: status("설정을 저장했습니다."), settings };
    }
    if (command === "open_log_folder") return status("로그 폴더를 열었습니다.");
    if (command === "check_for_updates" || command === "check_update_alert") {
      const version = update.appVersion || EMPTY_STATE.appVersion;
      update = {
        ...update,
        statusText: `최신 버전입니다. (${version})`,
        available: false,
        checking: false,
        releaseUrl: "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/latest",
        latestVersion: version,
      };
      const response = { status: status(update.statusText), update };
      return command === "check_update_alert"
        ? { ...response, shouldAlert: false, alertText: "" }
        : response;
    }
    if (command === "open_update_release") {
      return status(args.url ? "GitHub Release 페이지를 열었습니다." : "열 수 있는 릴리스 페이지가 없습니다.");
    }
    if (command === "open_repository") return status("GitHub 저장소를 엽니다. (미리보기)");
    if (command === "register_shutdown") {
      const input = args.input;
      shutdown =
        input.kind === "once"
          ? {
              ...shutdown,
              onceText: `${input.date} ${input.time} (미리보기)`,
              onceActive: true,
              onceDate: input.date,
              onceTime: input.time,
            }
          : {
              ...shutdown,
              weeklyText: `매주 ${input.days.map((day) => WEEKDAY_LABELS[day] ?? day).join("/")} ${input.time} (미리보기)`,
              weeklyActive: true,
              weeklyDays: input.days,
              weeklyTime: input.time,
            };
      return { status: status("예약 종료가 등록되었습니다."), shutdown };
    }
    if (command === "cancel_shutdown") {
      shutdown =
        args.kind === "once"
          ? { ...shutdown, onceText: "", onceActive: false, onceDate: null, onceTime: null }
          : { ...shutdown, weeklyText: "", weeklyActive: false, weeklyDays: [], weeklyTime: null };
      return { status: status("예약 종료가 취소되었습니다."), shutdown };
    }
    if (command === "get_app_state") {
      return {
        appVersion: EMPTY_STATE.appVersion,
        status: status("브라우저 미리보기 상태입니다."),
        control: EMPTY_STATE.control,
        settings,
        update,
        monitor,
      };
    }
    throw new Error(`지원하지 않는 브라우저 미리보기 command: ${command}`);
  };
}

export const browserPreviewPayload = createBrowserPreview();
