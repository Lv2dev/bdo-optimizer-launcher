import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  Bolt,
  Check,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  Eye,
  FolderOpen,
  Gauge,
  Laptop,
  Leaf,
  Monitor,
  Moon,
  Play,
  Plus,
  Power,
  RefreshCw,
  Repeat,
  RotateCcw,
  Settings,
  Sparkles,
  Sun,
  Trash2,
  X,
} from "lucide-react";
import "./styles.css";

const EMPTY_STATE = {
  appVersion: "0.1.0",
  status: {
    current: "초기화 중입니다.",
    previous: "",
  },
  control: {
    adminOk: false,
    gameRunning: false,
    currentMode: null,
    currentModeKnown: false,
    launcherPath: "",
  },
  schedule: {
    activeRuleInfo: "활성 규칙 없음.",
    rules: [],
    empty: true,
  },
  shutdown: {
    onceText: "",
    onceActive: false,
    weeklyText: "",
    weeklyActive: false,
  },
  settings: {
    themeMode: "system",
    effectiveDark: true,
    reduceMotion: false,
    autoTrayOnGameMinimize: false,
    closeToTray: true,
    autostartEnabled: false,
    autostartMinimized: false,
    launcherPath: "",
  },
  update: {
    statusText: "업데이트 채널 미설정.",
    available: false,
    checking: false,
    releaseUrl: "",
    appVersion: "0.1.0",
  },
  monitor: {
    running: false,
    pid: null,
    systemInfo: {
      cpuName: "Unknown CPU",
      gpuName: "Unknown GPU",
      gpuNames: [],
    },
    totals: {
      ramMb: 0,
      vramMb: 0,
    },
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

const MODE_META = {
  high: {
    label: "고성능",
    badge: "성능 우선",
    icon: Gauge,
  },
  normal: {
    label: "일반",
    badge: "균형",
    icon: Bolt,
  },
  low_power: {
    label: "저전력",
    badge: "절전",
    icon: Leaf,
  },
};

const KIND_META = {
  daily: "매일",
  weekday: "평일",
  weekend: "주말",
  specific_date: "특정일",
};

const WEEKDAYS = [
  ["MON", "월"],
  ["TUE", "화"],
  ["WED", "수"],
  ["THU", "목"],
  ["FRI", "금"],
  ["SAT", "토"],
  ["SUN", "일"],
];

const TABS = [
  { label: "제어", icon: Activity, enabled: true },
  { label: "스케줄", icon: Sparkles, enabled: true },
  { label: "모니터", icon: Monitor, enabled: true },
  { label: "설정", icon: Settings, enabled: true },
];

const GLASS_THEME = {
  mode: "dark",
  bg: "aurora",
  accent: ["#25d0c0", "#18a4e0", "#f0c04a"],
  blur: 30,
  frost: 0.12,
  radius: 22,
};

const ACCENTS = [
  ["#25d0c0", "#18a4e0", "#f0c04a"],
  ["#f0b53c", "#ff7a3d", "#ffd98a"],
  ["#7b8cff", "#5ad1ff", "#c4b6ff"],
  ["#ff6ea8", "#9b5cff", "#ffc4dd"],
];

const ACCENT_NAMES = ["청록", "골드", "인디고", "마젠타"];

let previewRules = [];
let previewShutdown = EMPTY_STATE.shutdown;
let previewSettings = EMPTY_STATE.settings;
let previewUpdate = EMPTY_STATE.update;
let previewMonitor = EMPTY_STATE.monitor;
let previewMonitorTick = 0;

function isTauriRuntime() {
  return Boolean(window.__TAURI_INTERNALS__);
}

function shouldUseNativeAppSurface() {
  return isTauriRuntime() || import.meta.env.DEV;
}

function syncRuntimeBodyMarkers() {
  const body = document.body;
  const nativeSurface = shouldUseNativeAppSurface();
  body.classList.toggle("tauri-runtime", isTauriRuntime());
  body.classList.toggle("native-app-surface", nativeSurface);
  body.dataset.runtime = isTauriRuntime() ? "tauri" : nativeSurface ? "dev" : "browser";
}

function todayValue() {
  const date = new Date();
  date.setMinutes(date.getMinutes() - date.getTimezoneOffset());
  return date.toISOString().slice(0, 10);
}

function currentWeekdayToken() {
  const mondayFirst = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
  return mondayFirst[new Date().getDay()];
}

function clampTimePart(value, max) {
  const numeric = Number.parseInt(value, 10);
  if (Number.isNaN(numeric)) {
    return 0;
  }
  return ((numeric % max) + max) % max;
}

function parseTimeParts(value) {
  const [hour = "0", minute = "0"] = String(value || "").split(":");
  return {
    hour: clampTimePart(hour, 24),
    minute: clampTimePart(minute, 60),
  };
}

function formatTimeParts(hour, minute) {
  return `${String(clampTimePart(hour, 24)).padStart(2, "0")}:${String(
    clampTimePart(minute, 60),
  ).padStart(2, "0")}`;
}

function stepTimeValue(value, field, delta) {
  const parts = parseTimeParts(value);
  if (field === "hour") {
    return formatTimeParts(parts.hour + delta, parts.minute);
  }
  return formatTimeParts(parts.hour, parts.minute + delta);
}

function scheduleSummary(input) {
  const kindLabel = input.kind === "specific_date" ? input.date : KIND_META[input.kind];
  return `${input.name} | ${kindLabel} | ${input.startTime}-${input.endTime} | ${MODE_META[input.mode].label}`;
}

function previewScheduleState() {
  const active = previewRules.find((rule) => rule.active);
  return {
    activeRuleInfo: active ? `활성 규칙: ${active.summary}` : "활성 규칙 없음.",
    rules: previewRules,
    empty: previewRules.length === 0,
  };
}

function previewMonitorState() {
  previewMonitorTick += 1;
  const wave = (base, amp, phase = 0) =>
    Math.max(0, Math.min(100, base + Math.sin(previewMonitorTick / 3 + phase) * amp));
  const cpuPct = wave(18, 9);
  const gpuPct = wave(34, 16, 1.2);
  const memMb = Math.round(8200 + Math.sin(previewMonitorTick / 5) * 260);
  const vramMb = Math.round(4100 + Math.sin(previewMonitorTick / 4 + 0.4) * 180);
  const ramMb = 32768;
  const vramTotalMb = 12288;
  previewMonitor = {
    running: true,
    pid: 4321,
    systemInfo: {
      cpuName: "AMD Ryzen 7 7800X3D 8-Core Processor",
      gpuName: "NVIDIA GeForce RTX 4080",
      gpuNames: ["NVIDIA GeForce RTX 4080"],
    },
    totals: {
      ramMb,
      vramMb: vramTotalMb,
    },
    metrics: {
      cpuPct,
      memMb,
      memPct: Math.min(100, (memMb / ramMb) * 100),
      gpuPct,
      vramMb,
      vramPct: Math.min(100, (vramMb / vramTotalMb) * 100),
      diskReadKbs: Math.round(120 + wave(60, 45, 2)),
      diskWriteKbs: Math.round(36 + wave(18, 12, 3)),
      fps: 144,
      fpsText: "144 FPS",
    },
    cores: Array.from({ length: 8 }, (_, index) => ({
      index,
      usagePct: wave(16 + index * 2, 18, index / 2),
      active: index < 8,
    })),
    statusText: "브라우저 미리보기 모니터링 중.",
  };
  return previewMonitor;
}

function browserPreviewPayload(command, args) {
  if (command === "apply_mode") {
    return {
      status: {
        current: `${MODE_META[args.mode].label} 모드 미리보기.`,
        previous: "",
      },
      control: {
        ...EMPTY_STATE.control,
        currentMode: args.mode,
        currentModeKnown: true,
      },
    };
  }

  if (command === "launch_game") {
    return {
      status: {
        current: "Tauri 앱에서 게임 실행 명령을 사용할 수 있습니다.",
        previous: "",
      },
      control: EMPTY_STATE.control,
    };
  }

  if (command === "refresh_game_status") {
    return {
      status: {
        current: "브라우저 미리보기 상태입니다.",
        previous: "",
      },
      control: EMPTY_STATE.control,
    };
  }

  if (command === "list_schedule_rules") {
    return previewScheduleState();
  }

  if (command === "add_schedule_rule") {
    const input = args.input;
    const ruleInput = {
      ...input,
      name: input.name.trim() || "이름 없는 규칙",
      date: input.kind === "specific_date" ? input.date : null,
    };
    const rule = {
      id: Date.now(),
      ...ruleInput,
      active: true,
      summary: scheduleSummary(ruleInput),
    };
    previewRules = [...previewRules, rule];
    return {
      status: { current: "스케줄 규칙이 추가되었습니다.", previous: "" },
      schedule: previewScheduleState(),
    };
  }

  if (command === "delete_schedule_rule") {
    previewRules = previewRules.filter((rule) => rule.id !== args.id);
    return {
      status: { current: "스케줄 규칙이 삭제되었습니다.", previous: "" },
      schedule: previewScheduleState(),
    };
  }

  if (command === "toggle_schedule_rule") {
    previewRules = previewRules.map((rule) =>
      rule.id === args.id ? { ...rule, active: !rule.active } : rule,
    );
    return {
      status: { current: "스케줄 규칙 상태가 변경되었습니다.", previous: "" },
      schedule: previewScheduleState(),
    };
  }

  if (command === "get_shutdown_state") {
    return previewShutdown;
  }

  if (command === "get_settings") {
    return previewSettings;
  }

  if (command === "get_monitor_snapshot") {
    return {
      monitor: previewMonitorState(),
    };
  }

  if (command === "set_setting") {
    const input = args.input;
    if (input.key === "theme_mode") {
      const themeMode = input.themeMode ?? previewSettings.themeMode;
      previewSettings = {
        ...previewSettings,
        themeMode,
        effectiveDark: themeMode === "system" ? true : themeMode === "dark",
      };
    } else if (input.key === "launcher_path") {
      previewSettings = {
        ...previewSettings,
        launcherPath: input.stringValue ?? "",
      };
    } else {
      const map = {
        reduce_motion: "reduceMotion",
        auto_tray_on_game_minimize: "autoTrayOnGameMinimize",
        close_to_tray: "closeToTray",
        autostart_enabled: "autostartEnabled",
        autostart_minimized: "autostartMinimized",
      };
      const field = map[input.key];
      if (field) {
        previewSettings = { ...previewSettings, [field]: Boolean(input.boolValue) };
        if (input.key === "autostart_enabled" && !input.boolValue) {
          previewSettings = { ...previewSettings, autostartMinimized: false };
        }
      }
    }
    return {
      status: { current: "설정을 저장했습니다.", previous: "" },
      settings: previewSettings,
    };
  }

  if (command === "open_log_folder") {
    return {
      status: { current: "로그 폴더를 열었습니다.", previous: "" },
    };
  }

  if (command === "check_for_updates") {
    previewUpdate = {
      ...previewUpdate,
      statusText: "업데이트 채널 미설정.",
      available: false,
      checking: false,
      releaseUrl: "",
    };
    return {
      status: { current: "업데이트 채널이 설정되지 않았습니다.", previous: "" },
      update: previewUpdate,
    };
  }

  if (command === "open_update_release") {
    return {
      status: {
        current: args.url ? "GitHub Release 페이지를 열었습니다." : "열 수 있는 릴리스 페이지가 없습니다.",
        previous: "",
      },
    };
  }

  if (command === "register_shutdown") {
    const input = args.input;
    if (input.kind === "once") {
      previewShutdown = {
        ...previewShutdown,
        onceText: `${input.date} ${input.time} (미리보기)`,
        onceActive: true,
      };
    } else {
      const days = input.days
        .map((day) => WEEKDAYS.find(([token]) => token === day)?.[1] ?? day)
        .join("/");
      previewShutdown = {
        ...previewShutdown,
        weeklyText: `매주 ${days} ${input.time} (미리보기)`,
        weeklyActive: true,
      };
    }
    return {
      status: { current: "예약 종료가 등록되었습니다.", previous: "" },
      shutdown: previewShutdown,
    };
  }

  if (command === "cancel_shutdown") {
    previewShutdown =
      args.kind === "once"
        ? { ...previewShutdown, onceText: "", onceActive: false }
        : { ...previewShutdown, weeklyText: "", weeklyActive: false };
    return {
      status: { current: "예약 종료가 취소되었습니다.", previous: "" },
      shutdown: previewShutdown,
    };
  }

  if (command === "get_app_state") {
    return {
      ...EMPTY_STATE,
      settings: previewSettings,
      update: previewUpdate,
      monitor: previewMonitor,
      status: {
        current: "브라우저 미리보기 상태입니다.",
        previous: "",
      },
    };
  }

  return {
    status: {
      current: "브라우저 미리보기 상태입니다.",
      previous: "",
    },
  };
}

function normalizePayload(command, payload) {
  if (command === "list_schedule_rules") {
    return { schedule: payload };
  }
  if (command === "get_shutdown_state") {
    return { shutdown: payload };
  }
  if (command === "get_settings") {
    return { settings: payload };
  }
  if (command === "get_monitor_snapshot") {
    return payload.monitor ? payload : { monitor: payload };
  }
  return payload;
}

function formatError(error) {
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  return "명령 처리 중 오류가 발생했습니다.";
}

function mergePayload(previous, payload) {
  return {
    appVersion: payload.appVersion ?? previous.appVersion,
    status: payload.status ?? previous.status,
    control: payload.control ?? previous.control,
    schedule: payload.schedule ?? previous.schedule,
    shutdown: payload.shutdown ?? previous.shutdown,
    settings: payload.settings ?? previous.settings,
    update: payload.update ?? previous.update,
    monitor: payload.monitor ?? previous.monitor,
  };
}

function Help({ tip }) {
  return (
    <span className="help" role="img" aria-label={tip} title={tip}>
      ?
    </span>
  );
}

function GlassSelect({ value, options, onChange, width }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);
  const [menuStyle, setMenuStyle] = useState({});

  const items = options.map((option) =>
    typeof option === "string" ? { value: option, label: option } : option,
  );
  const selected = items.find((item) => item.value === value) ?? items[0];

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const close = (event) => {
      if (!rootRef.current?.contains(event.target)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  useLayoutEffect(() => {
    if (!open || !rootRef.current) {
      return;
    }
    const rect = rootRef.current.getBoundingClientRect();
    setMenuStyle({
      left: rect.left,
      top: rect.bottom + 6,
      width: width ?? rect.width,
    });
  }, [open, width]);

  return (
    <div
      className="gsel"
      ref={rootRef}
      style={{ width }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          setOpen(false);
        }
      }}
    >
      <button
        type="button"
        className={`field gsel-btn${open ? " open" : ""}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="gsel-val">{selected?.label ?? value}</span>
        <ChevronDown className="gsel-chev" aria-hidden="true" />
      </button>
      {open ? (
        <div className="gsel-menu" style={menuStyle} role="listbox">
          {items.map((item) => (
            <button
              type="button"
              key={item.value}
              role="option"
              aria-selected={item.value === value}
              className={`gsel-opt${item.value === value ? " on" : ""}`}
              onClick={() => {
                onChange(item.value);
                setOpen(false);
              }}
            >
              <span>{item.label}</span>
              {item.value === value ? <Check aria-hidden="true" /> : null}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function PillSwitch({ value, options, onChange, style }) {
  const rootRef = useRef(null);
  const buttonRefs = useRef([]);
  const [indicator, setIndicator] = useState({ left: 4, width: 0 });

  const updateIndicator = useCallback(() => {
    const index = options.findIndex((option) => option.value === value);
    const button = buttonRefs.current[index];
    if (button && rootRef.current) {
      setIndicator({ left: button.offsetLeft, width: button.offsetWidth });
    }
  }, [options, value]);

  useLayoutEffect(() => {
    updateIndicator();
  }, [updateIndicator]);

  useEffect(() => {
    const resize = () => updateIndicator();
    window.addEventListener("resize", resize);
    const timer = window.setTimeout(updateIndicator, 50);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("resize", resize);
    };
  }, [updateIndicator]);

  return (
    <div className="pillswitch" ref={rootRef} style={style} role="group">
      <span className="pill-ind" style={indicator} aria-hidden="true" />
      {options.map((option, index) => (
        <button
          type="button"
          key={option.value}
          ref={(element) => {
            buttonRefs.current[index] = element;
          }}
          className={value === option.value ? "on" : ""}
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function StatusCard({ icon: Icon, label, value, meta, tone = "neutral" }) {
  return (
    <section className={`glass stat-card tone-${tone}`}>
      <div className="stat-top">
        <Icon aria-hidden="true" />
        <span>{label}</span>
      </div>
      <div className="stat-val">{value}</div>
      <div className="stat-meta">{meta}</div>
    </section>
  );
}

function WindowControls() {
  const runWindowCommand = async (action) => {
    if (!isTauriRuntime()) {
      return;
    }
    const appWindow = getCurrentWindow();
    await appWindow[action]();
  };

  return (
    <div className="win-ctrls">
      <button
        type="button"
        className="win-dot min"
        aria-label="창 최소화"
        onClick={() => runWindowCommand("minimize")}
      />
      <button
        type="button"
        className="win-dot max"
        aria-label="창 최대화 전환"
        onClick={() => runWindowCommand("toggleMaximize")}
      />
      <button
        type="button"
        className="win-dot close"
        aria-label="창 닫기"
        onClick={() => runWindowCommand("close")}
      />
    </div>
  );
}

function ControlTab({
  state,
  pending,
  onRefresh,
  onLaunch,
  onApplyMode,
}) {
  const mode = state.control.currentMode;
  const modeMeta = mode ? MODE_META[mode] : null;
  const canRunCommand = pending === null;
  const scheduleCount = state.schedule.rules.length;
  const shutdownMeta = state.shutdown.weeklyActive
    ? state.shutdown.weeklyText
    : state.shutdown.onceActive
      ? state.shutdown.onceText
      : state.schedule.activeRuleInfo;

  return (
    <main className="tab-panel ctrl-tab">
      <section className="stat-grid" aria-label="현재 상태" style={{ marginBottom: 14 }}>
        <div className="glass stat-card">
          <div className="stat-top">
            <span className="stat-dot" style={{ background: state.control.adminOk ? "var(--ok)" : "var(--warn)" }} />
            권한
          </div>
          <div className="stat-val" style={{ color: state.control.adminOk ? "var(--ok)" : "var(--warn)" }}>
            {state.control.adminOk ? "획득" : "미획득"}
          </div>
          <div className="stat-meta">{state.control.adminOk ? "관리자 권한 활성" : "관리자 권한 필요"}</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top">
            <span className="stat-dot" style={{ background: state.control.gameRunning ? "var(--ok)" : "var(--txt-faint)" }} />
            게임
          </div>
          <div className="stat-val" style={{ color: state.control.gameRunning ? "var(--ok)" : "var(--txt-dim)" }}>
            {state.control.gameRunning ? "실행 중" : "중지됨"}
          </div>
          <div className="stat-meta">BlackDesert64.exe</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top">
            <span className="stat-dot" style={{ background: "var(--accent)" }} />
            모드
          </div>
          <div className="stat-val" style={{ color: "var(--accent)" }}>
            {modeMeta ? `${modeMeta.label} 모드` : "대기"}
          </div>
          <div className="stat-meta">{modeMeta?.badge ?? "적용된 모드 없음"}</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top">
            <span className="stat-dot" style={{ background: "var(--info)" }} />
            스케줄
          </div>
          <div className="stat-val" style={{ color: "var(--info)" }}>
            {scheduleCount > 0 ? `${scheduleCount}개 규칙` : "규칙 없음"}
          </div>
          <div className="stat-meta">{shutdownMeta || "활성 규칙 없음"}</div>
        </div>
      </section>

      <section className="glass panel">
        <div className="section-label">
          <Play aria-hidden="true" />
          Game
          <Help tip="런처 실행과 프로세스 상태를 관리합니다" />
        </div>
        <p className="panel-sub">런처 실행과 프로세스 상태를 관리합니다.</p>
        <div className="row">
          <button type="button" className="btn" disabled={!canRunCommand} onClick={onRefresh}>
            <RefreshCw aria-hidden="true" />
            상태 새로고침
          </button>
          <button type="button" className="btn btn-primary" disabled={!canRunCommand} onClick={onLaunch}>
            <Play aria-hidden="true" />
            게임 실행
          </button>
        </div>
      </section>

      <section className="glass panel">
        <div className="section-label">
          <Gauge aria-hidden="true" />
          Mode
          <Help tip="우선순위와 CPU affinity를 즉시 적용합니다" />
        </div>
        <p className="panel-sub">BlackDesert64.exe 우선순위와 CPU affinity를 즉시 적용합니다.</p>
        <div className="seg" role="group" aria-label="최적화 모드">
          {Object.entries(MODE_META).map(([key, item]) => {
            const Icon = item.icon;
            const active = state.control.currentMode === key;
            return (
              <button
                type="button"
                key={key}
                className={`seg-btn${active ? " on" : ""}`}
                aria-pressed={active}
                disabled={!canRunCommand}
                onClick={() => onApplyMode(key)}
              >
                {active ? <span className="seg-badge">적용 중</span> : null}
                <Icon className="ico" aria-hidden="true" />
                <span>{item.label}</span>
              </button>
            );
          })}
        </div>
        <p className="legend">
          <b>고성능</b> 물리 코어 + High 우선순위 &nbsp;·&nbsp; <b>일반</b> 전체 코어 + Normal &nbsp;·&nbsp; <b>저전력</b> 마지막 코어 + Idle
        </p>
      </section>
    </main>
  );
}

function ScheduleTab({ state, pending, runCommand }) {
  const [openAuto, setOpenAuto] = useState(false);
  const [openRes, setOpenRes] = useState(true);
  const [name, setName] = useState("");
  const [kind, setKind] = useState("daily");
  const [date, setDate] = useState(todayValue);
  const [startTime, setStartTime] = useState("19:00");
  const [endTime, setEndTime] = useState("23:00");
  const [mode, setMode] = useState("high");
  const [shutdownKind, setShutdownKind] = useState("once");
  const [shutdownDate, setShutdownDate] = useState(todayValue);
  const [shutdownTime, setShutdownTime] = useState("23:30");
  const [days, setDays] = useState([currentWeekdayToken()]);
  const canRunCommand = pending === null;
  const shutdownParts = parseTimeParts(shutdownTime);
  const shutdownActive = state.shutdown.onceActive || state.shutdown.weeklyActive;
  const shutdownText = state.shutdown.weeklyActive
    ? state.shutdown.weeklyText
    : state.shutdown.onceActive
      ? state.shutdown.onceText
      : "";

  const submitSchedule = () => {
    runCommand("schedule-add", "add_schedule_rule", {
      input: {
        name: name || "이름 없는 규칙",
        kind,
        date: kind === "specific_date" ? date : null,
        startTime,
        endTime,
        mode,
      },
    });
    setName("");
  };

  const toggleDay = (token) => {
    setDays((current) =>
      current.includes(token) ? current.filter((day) => day !== token) : [...current, token],
    );
  };

  const stepShutdownTime = (field, delta) => {
    setShutdownTime((current) => stepTimeValue(current, field, delta));
  };

  const submitShutdown = () => {
    runCommand("shutdown-register", "register_shutdown", {
      input: {
        kind: shutdownKind,
        date: shutdownKind === "once" ? shutdownDate : null,
        time: shutdownTime,
        days: shutdownKind === "weekly" ? days : [],
      },
    });
  };

  const cancelShutdown = () => {
    runCommand("shutdown-cancel", "cancel_shutdown", {
      kind: state.shutdown.onceActive ? "once" : "weekly",
    });
  };

  const modeColor = (value) =>
    value === "high" ? "var(--warn)" : value === "low_power" ? "var(--ok)" : "var(--info)";

  return (
    <main className="tab-panel">
      <section className="glass panel">
        <button
          type="button"
          className={`acc-head${openAuto ? " open" : ""}`}
          aria-expanded={openAuto}
          onClick={() => setOpenAuto((open) => !open)}
        >
          <Repeat aria-hidden="true" />
          <h3>자동 모드 전환</h3>
          <span className="chip" style={{ marginLeft: "auto" }}>
            {state.schedule.empty ? "규칙 없음" : `${state.schedule.rules.length}개 활성`}
          </span>
          <ChevronDown className="acc-chev" aria-hidden="true" />
        </button>
        <div className={`acc-body${openAuto ? " open" : ""}`} style={{ height: openAuto ? "auto" : 0 }}>
          <div className="acc-inner">
            <div style={{ paddingTop: 14 }}>
              <p className="panel-sub" style={{ marginBottom: 14 }}>
                시간대별 자동 최적화 (앱 실행 중에만 동작)
              </p>
              <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                <input
                  className="field"
                  placeholder="규칙 이름 (예: 게임 시간 고성능)"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  maxLength={64}
                />
                <GlassSelect
                  value={kind}
                  onChange={setKind}
                  options={[
                    { value: "daily", label: "매일" },
                    { value: "weekday", label: "평일" },
                    { value: "weekend", label: "주말" },
                    { value: "specific_date", label: "특정일" },
                  ]}
                />
                {kind === "specific_date" ? (
                  <input className="field" type="date" value={date} onChange={(event) => setDate(event.target.value)} />
                ) : null}
                <div className="row">
                  <input
                    className="field"
                    placeholder="시작 HH:MM"
                    value={startTime}
                    onChange={(event) => setStartTime(event.target.value)}
                  />
                  <input
                    className="field"
                    placeholder="종료 HH:MM"
                    value={endTime}
                    onChange={(event) => setEndTime(event.target.value)}
                  />
                  <GlassSelect
                    value={mode}
                    onChange={setMode}
                    options={Object.entries(MODE_META).map(([value, item]) => ({
                      value,
                      label: item.label,
                    }))}
                  />
                </div>
                <button type="button" className="btn btn-primary btn-block" disabled={!canRunCommand} onClick={submitSchedule}>
                  <Plus aria-hidden="true" /> 규칙 추가
                </button>
              </div>

              {state.schedule.rules.length === 0 ? (
                <p className="empty">아직 등록된 규칙이 없습니다.</p>
              ) : (
                <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 9 }}>
                  {state.schedule.rules.map((rule) => (
                    <div
                      key={rule.id}
                      className="glass-2 rule-item"
                      style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 14px", opacity: rule.active ? 1 : 0.58 }}
                    >
                      <span className="stat-dot" style={{ background: modeColor(rule.mode) }} />
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 13, fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                          {rule.name}
                        </div>
                        <div style={{ fontSize: 11.5, color: "var(--txt-dim)", marginTop: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                          {rule.summary}
                        </div>
                      </div>
                      <span className="chip accent">{MODE_META[rule.mode].label}</span>
                      <button
                        type="button"
                        className="step-btn"
                        disabled={!canRunCommand}
                        aria-pressed={rule.active}
                        aria-label={`${rule.name} 규칙 ${rule.active ? "비활성화" : "활성화"}`}
                        onClick={() => runCommand("schedule-toggle", "toggle_schedule_rule", { id: rule.id })}
                        title={rule.active ? "비활성화" : "활성화"}
                      >
                        <Check aria-hidden="true" />
                      </button>
                      <button
                        type="button"
                        className="step-btn"
                        disabled={!canRunCommand}
                        aria-label={`${rule.name} 규칙 삭제`}
                        onClick={() => runCommand("schedule-delete", "delete_schedule_rule", { id: rule.id })}
                        title="삭제"
                      >
                        <Trash2 aria-hidden="true" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </section>

      <section className="glass panel">
        <button
          type="button"
          className={`acc-head${openRes ? " open" : ""}`}
          aria-expanded={openRes}
          onClick={() => setOpenRes((open) => !open)}
        >
          <Power aria-hidden="true" />
          <h3>PC 예약 종료</h3>
          <span className="chip warn" style={{ marginLeft: "auto" }}>
            전원 차단
          </span>
          <ChevronDown className="acc-chev" aria-hidden="true" />
        </button>
        <div className={`acc-body${openRes ? " open" : ""}`} style={{ height: openRes ? "auto" : 0 }}>
          <div className="acc-inner">
            <div style={{ paddingTop: 14 }}>
              {shutdownActive ? (
                <p className="panel-sub" style={{ color: "var(--warn)", marginBottom: 14 }}>
                  ● {shutdownText}
                </p>
              ) : (
                <p className="panel-sub" style={{ marginBottom: 14 }}>
                  지정한 시간에 PC를 자동으로 종료합니다.
                </p>
              )}

              <PillSwitch
                value={shutdownKind}
                onChange={setShutdownKind}
                style={{ marginBottom: 14 }}
                options={[
                  { value: "once", label: "단발 종료" },
                  { value: "weekly", label: "매주 반복" },
                ]}
              />

              {shutdownKind === "once" ? (
                <input
                  className="field"
                  type="date"
                  value={shutdownDate}
                  onChange={(event) => setShutdownDate(event.target.value)}
                  style={{ marginBottom: 16 }}
                />
              ) : (
                <div className="day-grid" style={{ marginBottom: 16 }}>
                  {WEEKDAYS.map(([token, label]) => (
                    <button
                      type="button"
                      key={token}
                      className={`day-btn${days.includes(token) ? " on" : ""}`}
                      aria-pressed={days.includes(token)}
                      onClick={() => toggleDay(token)}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              )}

              <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: "0.08em", color: "var(--txt-faint)", marginBottom: 12 }}>
                종료 시간
              </div>
              <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 18, marginBottom: 18 }} aria-label="종료 시간">
                <div className="stepper">
                  <div className="lab">시</div>
                  <button type="button" className="step-btn" onClick={() => stepShutdownTime("hour", 1)}>
                    <ChevronUp aria-hidden="true" />
                  </button>
                  <div className="num">{String(shutdownParts.hour).padStart(2, "0")}</div>
                  <button type="button" className="step-btn" onClick={() => stepShutdownTime("hour", -1)}>
                    <ChevronDown aria-hidden="true" />
                  </button>
                </div>
                <div className="time-separator">:</div>
                <div className="stepper">
                  <div className="lab">분</div>
                  <button type="button" className="step-btn" onClick={() => stepShutdownTime("minute", 5)}>
                    <ChevronUp aria-hidden="true" />
                  </button>
                  <div className="num">{String(shutdownParts.minute).padStart(2, "0")}</div>
                  <button type="button" className="step-btn" onClick={() => stepShutdownTime("minute", -5)}>
                    <ChevronDown aria-hidden="true" />
                  </button>
                </div>
              </div>

              <div className="row">
                <button type="button" className="btn btn-warn" disabled={!canRunCommand} onClick={submitShutdown}>
                  <Check aria-hidden="true" /> 예약 등록
                </button>
                <button type="button" className="btn" disabled={!canRunCommand || !shutdownActive} onClick={cancelShutdown}>
                  예약 취소
                </button>
              </div>
            </div>
          </div>
        </div>
      </section>
    </main>
  );
}

function ToggleSwitch({ checked, disabled = false, onToggle, label }) {
  return (
    <button
      type="button"
      className={`switch${checked ? " on" : ""}`}
      aria-pressed={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onToggle}
    >
      <i aria-hidden="true" />
    </button>
  );
}

function formatPercent(value) {
  return value === null || value === undefined ? "--" : `${Math.round(value)}%`;
}

function formatKbs(value) {
  return value === null || value === undefined ? "--" : `${value} KB/s`;
}

function formatGbOfTotal(usedMb, totalMb) {
  if (usedMb === null || usedMb === undefined) {
    return "--";
  }
  const usedGb = usedMb / 1024;
  if (!totalMb) {
    return `${usedGb.toFixed(1)} GB`;
  }
  return `${usedGb.toFixed(1)} / ${(totalMb / 1024).toFixed(1)} GB`;
}

function formatMbOfTotal(usedMb, totalMb) {
  if (usedMb === null || usedMb === undefined) {
    return "--";
  }
  return totalMb ? `${usedMb} / ${totalMb} MB` : `${usedMb} MB`;
}

function emptyMonitorSeries() {
  return {
    cpu: Array(40).fill(0),
    mem: Array(40).fill(0),
    gpu: Array(40).fill(0),
    vram: Array(40).fill(0),
  };
}

function pushSeries(series, value) {
  return [...series.slice(1), Math.max(0, Math.min(100, Number(value) || 0))];
}

function smoothPath(values, width, height, maxValue) {
  const max = maxValue > 0 ? maxValue : 1;
  if (values.length < 2) {
    return { line: "", area: "" };
  }
  const points = values.map((value, index) => [
    (index / (values.length - 1)) * width,
    height - (Math.min(value, max) / max) * (height - 6) - 3,
  ]);
  let line = `M ${points[0][0]} ${points[0][1]}`;
  for (let index = 0; index < points.length - 1; index += 1) {
    const [x0, y0] = points[index];
    const [x1, y1] = points[index + 1];
    const cx = (x0 + x1) / 2;
    line += ` C ${cx} ${y0} ${cx} ${y1} ${x1} ${y1}`;
  }
  return {
    line,
    area: `${line} L ${width} ${height} L 0 ${height} Z`,
  };
}

function graphGradientId(name) {
  return `g_${Array.from(name)
    .map((ch) => ch.charCodeAt(0).toString(16))
    .join("")}`;
}

function LiveGraph({ name, color, data, valueLabel, foot }) {
  const width = 320;
  const height = 76;
  const { line, area } = smoothPath(data, width, height, 100);
  const current = data[data.length - 1] || 0;
  const gid = graphGradientId(name);
  const y = height - (Math.min(current, 100) / 100) * (height - 6) - 3;

  return (
    <div className="glass graph-card">
      <div className="graph-head">
        <div className="graph-name" style={{ color }}>
          {name}
        </div>
        <div className="graph-val" style={{ color }}>
          {valueLabel}
        </div>
      </div>
      <svg className="graph-svg" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden="true">
        <defs>
          <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.42" />
            <stop offset="100%" stopColor={color} stopOpacity="0.02" />
          </linearGradient>
        </defs>
        {[0.25, 0.5, 0.75].map((grid) => (
          <line key={grid} x1="0" y1={height * grid} x2={width} y2={height * grid} stroke="var(--glass-stroke)" strokeWidth="0.5" />
        ))}
        <path d={area} fill={`url(#${gid})`} />
        <path d={line} fill="none" stroke={color} strokeWidth="2" strokeLinecap="round" />
        <circle cx={width} cy={y} r="3.2" fill={color} />
      </svg>
      <div className="graph-foot">
        <span>{foot[0]}</span>
        <span>{foot[1]}</span>
      </div>
    </div>
  );
}

function MonitorTab({ state }) {
  const monitor = state.monitor ?? EMPTY_STATE.monitor;
  const metrics = monitor.metrics ?? EMPTY_STATE.monitor.metrics;
  const totals = monitor.totals ?? EMPTY_STATE.monitor.totals;
  const systemInfo = monitor.systemInfo ?? EMPTY_STATE.monitor.systemInfo;
  const [view, setView] = useState("bars");
  const [series, setSeries] = useState(emptyMonitorSeries);

  useEffect(() => {
    setSeries((current) => ({
      cpu: pushSeries(current.cpu, metrics.cpuPct ?? 0),
      mem: pushSeries(current.mem, metrics.memPct ?? 0),
      gpu: pushSeries(current.gpu, metrics.gpuPct ?? 0),
      vram: pushSeries(current.vram, metrics.vramPct ?? 0),
    }));
  }, [metrics.cpuPct, metrics.memPct, metrics.gpuPct, metrics.vramPct]);

  const cores = Array.isArray(monitor.cores) ? monitor.cores : [];

  return (
    <main className="tab-panel">
      <section className="glass panel" style={{ paddingTop: 14, paddingBottom: 14 }}>
        <div className="hw-row">
          <Activity aria-hidden="true" style={{ color: "var(--accent)" }} />
          <span className="hw-key">CPU</span>
          <span className="hw-val">{systemInfo.cpuName || "Unknown CPU"}</span>
        </div>
        <div className="hw-row">
          <Monitor aria-hidden="true" style={{ color: "var(--accent-2)" }} />
          <span className="hw-key">GPU</span>
          <span className="hw-val">{systemInfo.gpuName || "Unknown GPU"}</span>
        </div>
      </section>
      <section className="glass panel">
        <div className="panel-head">
          <Sparkles aria-hidden="true" style={{ color: "var(--accent-3)" }} />
          <h3>검은사막 자원 모니터</h3>
          <PillSwitch
            value={view}
            onChange={setView}
            style={{ marginLeft: "auto", width: 168 }}
            options={[
              { value: "bars", label: "그래프" },
              { value: "cores", label: "코어별" },
            ]}
          />
        </div>
        <p className="panel-sub">{monitor.statusText || "모니터 샘플 대기 중."}</p>

        {view === "bars" ? (
          <div className="monitor-graphs">
            <LiveGraph
              name="CPU"
              color="#3fd0ff"
              data={series.cpu}
              valueLabel={formatPercent(metrics.cpuPct)}
              foot={["0 - 100 %", monitor.pid ? `PID ${monitor.pid}` : "대기"]}
            />
            <LiveGraph
              name="메모리"
              color="#b48cff"
              data={series.mem}
              valueLabel={formatGbOfTotal(metrics.memMb, totals.ramMb)}
              foot={["사용 / 총", `R ${formatKbs(metrics.diskReadKbs)}`]}
            />
            <LiveGraph
              name="GPU"
              color="#34e0a1"
              data={series.gpu}
              valueLabel={formatPercent(metrics.gpuPct)}
              foot={["0 - 100 %", metrics.fpsText || "세션 미시작"]}
            />
            <LiveGraph
              name="VRAM"
              color="#ff9b6e"
              data={series.vram}
              valueLabel={formatMbOfTotal(metrics.vramMb, totals.vramMb)}
              foot={["사용 / 총", `W ${formatKbs(metrics.diskWriteKbs)}`]}
            />
          </div>
        ) : (
          <div>
            {cores.length === 0 ? (
              <p className="empty">코어 샘플 대기 중입니다.</p>
            ) : (
              <div className="core-grid">
                {cores.map((core) => (
                  <div key={core.index} className={`core-cell${core.active ? " active" : ""}`}>
                    <div className="ci">CORE {core.index}</div>
                    <div className="cv" style={{ color: core.usagePct > 60 ? "var(--warn)" : "var(--accent)" }}>
                      {formatPercent(core.usagePct)}
                    </div>
                    <div className="core-bar">
                      <i style={{ width: `${Math.max(0, Math.min(100, core.usagePct || 0))}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            )}
            <p className="legend" style={{ marginTop: 14 }}>
              활성 affinity 코어는 강조 표시됩니다. FPS: <b>{metrics.fpsText || "세션 미시작"}</b>
            </p>
          </div>
        )}
      </section>
    </main>
  );
}

function SettingsTab({ state, pending, runCommand, accent, onAccent, showToast }) {
  const settings = state.settings;
  const update = state.update;
  const canRunCommand = pending === null;
  const accArr = Array.isArray(accent) ? accent : [accent];

  const setSetting = (key, values) => {
    runCommand(`setting-${key}`, "set_setting", {
      input: {
        key,
        themeMode: null,
        boolValue: null,
        stringValue: null,
        ...values,
      },
    });
  };

  const themeOptions = [
    { value: "light", icon: Sun, label: "라이트 모드" },
    { value: "dark", icon: Moon, label: "다크 모드" },
    { value: "system", icon: Laptop, label: "자동 (OS 설정)", desc: "Windows 앱 테마 설정을 따릅니다." },
  ];

  return (
    <main className="tab-panel">
      <section className="glass panel">
        <div className="set-head">
          테마
          <Help tip="앱 표시 색상을 변경합니다" />
        </div>
        <p className="panel-sub">앱 표시 색상을 변경합니다.</p>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }} role="group" aria-label="테마 모드">
          {themeOptions.map((option) => {
            const Icon = option.icon;
            const active = settings.themeMode === option.value;
            return (
              <button
                type="button"
                key={option.value}
                className={"glass-2 theme-card" + (active ? " on" : "")}
                aria-pressed={active}
                disabled={!canRunCommand}
                onClick={() => setSetting("theme_mode", { themeMode: option.value })}
              >
                <Icon className="tc-ico" aria-hidden="true" />
                <div className="tc-body">
                  <div className="tc-t">{option.label}</div>
                  {option.desc ? <div className="tc-d">{option.desc}</div> : null}
                </div>
                {active ? <Check className="tc-check" aria-hidden="true" /> : null}
              </button>
            );
          })}
        </div>

        <div style={{ height: 1, background: "var(--glass-stroke)", margin: "16px 0 14px" }} />
        <div className="set-head" style={{ fontSize: 12.5 }}>
          액센트 색상
        </div>
        <p className="panel-sub" style={{ marginBottom: 12 }}>
          버튼·그래프·강조 요소에 쓰이는 포인트 색입니다.
        </p>
        <div className="accent-grid">
          {ACCENTS.map((palette, index) => {
            const active = String(accArr[0]).toLowerCase() === String(palette[0]).toLowerCase();
            return (
              <button
                type="button"
                key={palette[0]}
                className={`accent-swatch${active ? " on" : ""}`}
                aria-pressed={active}
                title={ACCENT_NAMES[index]}
                onClick={() => {
                  onAccent(palette);
                  showToast(`${ACCENT_NAMES[index]} 액센트 적용`);
                }}
              >
                <span className="sw-colors">
                  {palette.slice(0, 3).map((color) => (
                    <i key={color} style={{ background: color }} />
                  ))}
                </span>
                <span className="sw-name">{ACCENT_NAMES[index]}</span>
                {active ? <Check className="sw-check" aria-hidden="true" /> : null}
              </button>
            );
          })}
        </div>
      </section>

      <section className="glass panel">
        <div className="set-head">
          <Eye aria-hidden="true" style={{ opacity: 0.7 }} />
          접근성
          <Help tip="모션을 줄여 시각적 피로를 낮춥니다" />
        </div>
        <p className="panel-sub">
          애니메이션 줄이기를 켜면 신규 모션이 비활성화됩니다. 배터리 절약과 시각적 피로 감소에 도움이 됩니다.
        </p>
        <div className="set-row">
          <div className="meta">
            <div className="t">애니메이션 줄이기</div>
            <div className="d">{settings.reduceMotion ? "켜짐" : "꺼짐"}</div>
          </div>
          <ToggleSwitch
            checked={settings.reduceMotion}
            disabled={!canRunCommand}
            label="애니메이션 줄이기"
            onToggle={() =>
              setSetting("reduce_motion", { boolValue: !settings.reduceMotion })
            }
          />
        </div>
      </section>

      <section className="glass panel">
        <div className="set-head">
          런처 동작
          <Help tip="게임/창 상태에 따른 자동 동작" />
        </div>
        <div className="set-row" style={{ paddingTop: 4 }}>
          <div className="meta">
            <div className="t">게임이 트레이로 내려가면 자동 저전력 모드</div>
            <div className="d">검은사막 창이 숨겨지면 저전력 모드 적용, 다시 나타나면 직전 모드로 복원합니다.</div>
          </div>
          <ToggleSwitch
            checked={settings.autoTrayOnGameMinimize}
            disabled={!canRunCommand}
            label="게임 트레이 진입 시 저전력"
            onToggle={() =>
              setSetting("auto_tray_on_game_minimize", {
                boolValue: !settings.autoTrayOnGameMinimize,
              })
            }
          />
        </div>
        <div className="set-row">
          <div className="meta">
            <div className="t">창 닫기 시 트레이로 숨기기</div>
            <div className="d">끄면 X 버튼 클릭 시 앱이 완전히 종료됩니다.</div>
          </div>
          <ToggleSwitch
            checked={settings.closeToTray}
            disabled={!canRunCommand}
            label="창 닫기 시 트레이로 숨기기"
            onToggle={() => setSetting("close_to_tray", { boolValue: !settings.closeToTray })}
          />
        </div>
      </section>

      <section className="glass panel">
        <div className="set-head">
          시작 옵션
          <Help tip="Windows 로그온 시 동작" />
        </div>
        <p className="panel-sub">
          Windows 로그온 시 자동으로 실행합니다. 작업 스케줄러에 등록되어 UAC 프롬프트 없이 승격 실행됩니다.
        </p>
        <div className="set-row" style={{ paddingTop: 4 }}>
          <div className="meta">
            <div className="t" style={{ color: settings.autostartEnabled ? "var(--accent)" : "var(--txt)" }}>
              Windows 시작 시 자동 실행
            </div>
          </div>
          <ToggleSwitch
            checked={settings.autostartEnabled}
            disabled={!canRunCommand}
            label="Windows 시작 시 자동 실행"
            onToggle={() =>
              setSetting("autostart_enabled", { boolValue: !settings.autostartEnabled })
            }
          />
        </div>
        <div className="set-row">
          <div className="meta">
            <div className="t" style={{ color: settings.autostartMinimized ? "var(--accent)" : "var(--txt)" }}>
              자동 실행 시 트레이로 시작
            </div>
            <div className="d">켜면 부팅 시 창을 띄우지 않고 트레이에만 상주합니다.</div>
          </div>
          <ToggleSwitch
            checked={settings.autostartMinimized}
            disabled={!canRunCommand || !settings.autostartEnabled}
            label="자동 실행 시 트레이로 시작"
            onToggle={() =>
              setSetting("autostart_minimized", { boolValue: !settings.autostartMinimized })
            }
          />
        </div>
      </section>

      <section className="glass panel">
        <div className="set-head">
          <FolderOpen aria-hidden="true" style={{ opacity: 0.7 }} />
          런처 경로
          <Help tip="검은사막 실행 파일 경로" />
        </div>
        <p className="panel-sub">저장된 경로가 없으면 자동으로 탐색합니다.</p>
        <div className="path-field readonly">
          <FolderOpen aria-hidden="true" style={{ opacity: 0.6, flexShrink: 0 }} />
          <span className={settings.launcherPath ? "mono" : ""}>
            {settings.launcherPath || "저장된 경로 없음 (자동 탐색)"}
          </span>
        </div>
        <button
          type="button"
          className="btn btn-block"
          style={{ marginTop: 11 }}
          disabled={!canRunCommand || !settings.launcherPath}
          onClick={() => setSetting("launcher_path", { stringValue: "" })}
        >
          <RotateCcw aria-hidden="true" /> 경로 초기화
        </button>
      </section>

      <section className="glass panel">
        <div className="set-head">
          <ExternalLink aria-hidden="true" style={{ opacity: 0.7 }} />
          업데이트
          <Help tip="GitHub Release에서 새 버전을 확인합니다" />
        </div>
        <p className="panel-sub">새 릴리스가 있으면 GitHub 릴리스 페이지로 이동합니다.</p>
        <div className={`update-box${update.available ? " available" : ""}`}>
          <span>{update.statusText}</span>
          <strong>v{update.appVersion}</strong>
        </div>
        <div className="row">
          <button
            type="button"
            className="btn"
            disabled={!canRunCommand}
            onClick={() => runCommand("update-check", "check_for_updates")}
          >
            <RefreshCw aria-hidden="true" />
            업데이트 확인
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!canRunCommand || !update.available || !update.releaseUrl}
            onClick={() =>
              runCommand("update-open", "open_update_release", { url: update.releaseUrl })
            }
          >
            <ExternalLink aria-hidden="true" />
            릴리스 열기
          </button>
        </div>
      </section>

      <section className="glass panel">
        <div className="set-head">
          <FolderOpen aria-hidden="true" style={{ opacity: 0.7 }} />
          진단
          <Help tip="로그 파일로 문제를 진단합니다" />
        </div>
        <p className="panel-sub">버그 보고 시 로그 파일을 첨부하면 진단에 도움이 됩니다.</p>
        <button
          type="button"
          className="btn btn-block"
          disabled={!canRunCommand}
          onClick={() => runCommand("log-open", "open_log_folder")}
        >
          <FolderOpen aria-hidden="true" />
          로그 폴더 열기
        </button>
      </section>

      <div className="app-footer">bdo-optimizer-launcher v{state.appVersion}</div>
    </main>
  );
}

function App() {
  const [state, setState] = useState(EMPTY_STATE);
  const [launcherPath, setLauncherPath] = useState("");
  const [pending, setPending] = useState(null);
  const [activeTab, setActiveTab] = useState(0);
  const [accent, setAccent] = useState(GLASS_THEME.accent);
  const [toast, setToast] = useState({ show: false, message: "" });
  const toastTimer = useRef(null);
  const monitorPollInFlight = useRef(false);
  const tabRefs = useRef([]);
  const [pill, setPill] = useState({ left: 0, width: 0 });

  const showToast = useCallback((message) => {
    setToast({ show: true, message });
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => {
      setToast((current) => ({ ...current, show: false }));
    }, 2600);
  }, []);

  const applyPayload = useCallback(
    (payload, shouldToast = false) => {
      setState((current) => mergePayload(current, payload));
      if (payload.control?.launcherPath !== undefined) {
        setLauncherPath((typed) => typed || payload.control.launcherPath || "");
      }
      if (payload.settings?.launcherPath !== undefined) {
        setLauncherPath(payload.settings.launcherPath || "");
      }
      if (shouldToast && payload.status?.current) {
        showToast(payload.status.current);
      }
    },
    [showToast],
  );

  const runCommand = useCallback(
    async (name, command, args, shouldToast = true, trackPending = true) => {
      if (trackPending) {
        setPending(name);
      }
      try {
        const rawPayload = isTauriRuntime()
          ? await invoke(command, args)
          : browserPreviewPayload(command, args ?? {});
        applyPayload(normalizePayload(command, rawPayload), shouldToast);
      } catch (error) {
        const message = formatError(error);
        setState((current) => ({
          ...current,
          status: {
            current: message,
            previous: current.status.current,
          },
        }));
        if (shouldToast) {
          showToast(message);
        }
      } finally {
        if (trackPending) {
          setPending(null);
        }
      }
    },
    [applyPayload, showToast],
  );

  const reloadSchedule = useCallback(async () => {
    await runCommand("schedule-load", "list_schedule_rules", undefined, false);
    await runCommand("shutdown-load", "get_shutdown_state", undefined, false);
  }, [runCommand]);

  const recalcTabPill = useCallback(() => {
    const activeButton = tabRefs.current[activeTab];
    if (activeButton) {
      setPill({ left: activeButton.offsetLeft, width: activeButton.offsetWidth });
    }
  }, [activeTab]);

  useEffect(() => {
    const body = document.body;
    body.dataset.mode = state.settings.effectiveDark ? "dark" : "light";
    body.dataset.bg = state.settings.effectiveDark ? GLASS_THEME.bg : "frost";
    body.classList.toggle("reduce-motion", state.settings.reduceMotion);
    body.style.setProperty("--accent", accent[0]);
    body.style.setProperty("--accent-2", accent[1] ?? accent[0]);
    body.style.setProperty("--accent-3", accent[2] ?? accent[0]);
    body.style.setProperty("--blur", `${GLASS_THEME.blur}px`);
    body.style.setProperty("--frost", String(GLASS_THEME.frost));
    body.style.setProperty("--radius", `${GLASS_THEME.radius}px`);
  }, [accent, state.settings.effectiveDark, state.settings.reduceMotion]);

  useEffect(() => {
    runCommand("init", "get_app_state", undefined, false);
  }, [runCommand]);

  useLayoutEffect(() => {
    recalcTabPill();
  }, [recalcTabPill]);

  useEffect(() => {
    const onResize = () => recalcTabPill();
    const timer = window.setTimeout(recalcTabPill, 60);
    window.addEventListener("resize", onResize);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("resize", onResize);
    };
  }, [recalcTabPill]);

  useEffect(() => {
    if (activeTab === 1) {
      reloadSchedule();
    }
    if (activeTab === 3) {
      runCommand("settings-load", "get_settings", undefined, false);
    }
  }, [activeTab, reloadSchedule, runCommand]);

  useEffect(() => {
    if (activeTab !== 2) {
      return undefined;
    }

    const pollMonitor = async () => {
      if (monitorPollInFlight.current) {
        return;
      }
      monitorPollInFlight.current = true;
      try {
        await runCommand("monitor-sample", "get_monitor_snapshot", undefined, false, false);
      } finally {
        monitorPollInFlight.current = false;
      }
    };

    pollMonitor();
    const timer = window.setInterval(pollMonitor, 1000);
    return () => {
      window.clearInterval(timer);
      // 모니터 탭을 벗어나면 백엔드 ETW FPS 세션을 능동적으로 중단한다.
      if (isTauriRuntime()) {
        invoke("stop_monitor_session").catch(() => {});
      }
    };
  }, [activeTab, runCommand]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (pending === null) {
        runCommand("refresh", "refresh_game_status", undefined, false, false);
      }
    }, 5000);
    return () => window.clearInterval(timer);
  }, [pending, runCommand]);

  const titleMode = state.control.currentMode
    ? MODE_META[state.control.currentMode].label
    : "대기";

  return (
    <>
      <div className="bg-stage" aria-hidden="true">
        <div className="orb orb-1" />
        <div className="orb orb-2" />
        <div className="orb orb-3" />
        <div className="orb orb-4" />
      </div>
      <div className="bg-grain" aria-hidden="true" />
      <div className="glass app-window">
        <header className="titlebar" data-tauri-drag-region>
          <div className="title-id" data-tauri-drag-region>
            <div className="app-mark" aria-hidden="true">
              <Bolt />
            </div>
            <div className="app-title" data-tauri-drag-region>
              BDO Optimizer <span>{titleMode}</span>
            </div>
          </div>
          <WindowControls />
        </header>

        <nav className="tabbar" aria-label="주요 탭" role="tablist">
          <div className="tab-pill" style={{ left: pill.left, width: pill.width }} />
          {TABS.map((tab, index) => {
            const Icon = tab.icon;
            return (
              <button
                type="button"
                key={tab.label}
                role="tab"
                aria-selected={index === activeTab}
                ref={(element) => {
                  tabRefs.current[index] = element;
                }}
                className={`tab${index === activeTab ? " active" : ""}`}
                disabled={!tab.enabled}
                onClick={() => setActiveTab(index)}
              >
                <Icon aria-hidden="true" />
                {tab.label}
              </button>
            );
          })}
        </nav>

        <div className="content" key={activeTab} role="tabpanel">
          {activeTab === 0 ? (
            <ControlTab
              state={state}
              pending={pending}
              onRefresh={() => runCommand("refresh", "refresh_game_status")}
              onLaunch={() => runCommand("launch", "launch_game", { launcherPath })}
              onApplyMode={(mode) => runCommand("mode", "apply_mode", { mode })}
            />
          ) : activeTab === 1 ? (
            <ScheduleTab
              state={state}
              pending={pending}
              runCommand={runCommand}
            />
          ) : activeTab === 2 ? (
            <MonitorTab state={state} />
          ) : (
            <SettingsTab
              state={state}
              pending={pending}
              runCommand={runCommand}
              accent={accent}
              onAccent={setAccent}
              showToast={showToast}
            />
          )}
        </div>

        <footer className="statusbar">
          <span className={`status-led${state.control.gameRunning ? " on" : ""}`} />
          <span>{state.status.current}</span>
          <span className="status-side">{state.control.gameRunning ? "게임 실행 중" : "대기 중"}</span>
        </footer>

        <div className={`toast${toast.show ? " show" : ""}`} role="status" aria-live="polite">
          <span className="status-led on" />
          {toast.message}
        </div>
      </div>
    </>
  );
}

const rootElement = document.getElementById("root");
syncRuntimeBodyMarkers();
const root = window.__BDO_OPTIMIZER_ROOT__ ?? createRoot(rootElement);
window.__BDO_OPTIMIZER_ROOT__ = root;
root.render(<App />);
