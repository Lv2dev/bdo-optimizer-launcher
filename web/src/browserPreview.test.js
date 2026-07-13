import { describe, expect, test } from "vitest";
import { createBrowserPreview } from "./browserPreview.js";
import { normalizePayload } from "./payload.js";

const DIRECT_DTO_KEYS = {
  get_shutdown_state: [
    "onceText",
    "onceActive",
    "onceDate",
    "onceTime",
    "weeklyText",
    "weeklyActive",
    "weeklyDays",
    "weeklyTime",
  ],
  get_settings: [
    "themeMode",
    "effectiveDark",
    "accentPalette",
    "reduceMotion",
    "autoTrayOnGameMinimize",
    "closeToTray",
    "autostartEnabled",
    "autostartMinimized",
    "launcherPath",
    "defaultMode",
    "monitorIntervalMs",
    "updateAlertEnabled",
    "updateCheckIntervalMs",
  ],
  get_monitor_snapshot: ["running", "pid", "systemInfo", "totals", "metrics", "cores", "statusText"],
};

const MONITOR_NESTED_DTO_KEYS = {
  systemInfo: ["cpuName", "gpuName", "gpuNames"],
  totals: ["ramMb", "vramMb"],
  metrics: [
    "cpuPct",
    "memMb",
    "memPct",
    "gpuPct",
    "vramMb",
    "vramPct",
    "diskReadKbs",
    "diskWriteKbs",
    "fps",
    "fpsText",
  ],
  core: ["index", "usagePct", "active"],
};

describe("browser preview의 실제 Tauri raw shape", () => {
  test("직접 DTO command는 wrapper 없이 반환한다", () => {
    const preview = createBrowserPreview();
    const monitor = preview("get_monitor_snapshot");
    const settings = preview("get_settings");
    const status = preview("open_repository");

    expect(monitor.running).toBe(true);
    expect(monitor.monitor).toBeUndefined();
    expect(settings.themeMode).toBe("system");
    expect(settings.settings).toBeUndefined();
    expect(status.current).toContain("GitHub");
    expect(status.status).toBeUndefined();
  });

  test("get_app_state는 Rust AppStateDto의 여섯 필드만 반환한다", () => {
    const preview = createBrowserPreview();
    expect(Object.keys(preview("get_app_state")).sort()).toEqual(
      ["appVersion", "control", "monitor", "settings", "status", "update"].sort(),
    );
  });

  test("정규화 후 direct monitor와 status가 공용 payload에 합쳐진다", () => {
    const preview = createBrowserPreview();
    expect(normalizePayload("get_monitor_snapshot", preview("get_monitor_snapshot")).monitor.running).toBe(
      true,
    );
    expect(normalizePayload("open_repository", preview("open_repository")).status.current).toContain(
      "GitHub",
    );
  });

  test("monitor direct DTO의 중첩 camelCase key를 Rust 계약으로 고정한다", () => {
    const monitor = createBrowserPreview()("get_monitor_snapshot");
    expect(Object.keys(monitor.systemInfo).sort()).toEqual([...MONITOR_NESTED_DTO_KEYS.systemInfo].sort());
    expect(Object.keys(monitor.totals).sort()).toEqual([...MONITOR_NESTED_DTO_KEYS.totals].sort());
    expect(Object.keys(monitor.metrics).sort()).toEqual([...MONITOR_NESTED_DTO_KEYS.metrics].sort());
    expect(Object.keys(monitor.cores[0]).sort()).toEqual([...MONITOR_NESTED_DTO_KEYS.core].sort());
  });

  test("미지원 command는 성공처럼 위장하지 않는다", () => {
    const preview = createBrowserPreview();
    expect(() => preview("typo_command")).toThrow(/지원하지 않는/);
  });

  test("schedule id와 void command 계약도 Rust DTO를 따른다", () => {
    const preview = createBrowserPreview();
    const input = {
      name: "규칙",
      kind: "daily",
      date: null,
      startTime: "19:00",
      endTime: "23:00",
      mode: "high",
    };
    expect(preview("add_schedule_rule", { input }).schedule.rules[0].id).toBe(1);
    expect(preview("add_schedule_rule", { input }).schedule.rules[1].id).toBe(2);
    expect(preview("stop_monitor_session")).toBeNull();
  });

  test("command별 top-level raw key를 고정한다", () => {
    const input = {
      name: "규칙",
      kind: "daily",
      date: null,
      startTime: "19:00",
      endTime: "23:00",
      mode: "high",
    };
    const cases = [
      ["apply_mode", { mode: "high" }, ["control", "status"]],
      ["launch_game", {}, ["control", "status"]],
      ["refresh_game_status", {}, ["control", "status"]],
      ["list_schedule_rules", {}, ["activeRuleInfo", "empty", "rules"]],
      ["add_schedule_rule", { input }, ["schedule", "status"]],
      ["delete_schedule_rule", { id: 1 }, ["schedule", "status"]],
      ["toggle_schedule_rule", { id: 1 }, ["schedule", "status"]],
      ["get_shutdown_state", {}, DIRECT_DTO_KEYS.get_shutdown_state],
      ["get_settings", {}, DIRECT_DTO_KEYS.get_settings],
      ["get_monitor_snapshot", {}, DIRECT_DTO_KEYS.get_monitor_snapshot],
      ["set_setting", { input: { key: "accent_palette", intValue: 2 } }, ["settings", "status"]],
      ["open_log_folder", {}, ["current", "previous"]],
      ["check_for_updates", {}, ["status", "update"]],
      ["check_update_alert", {}, ["alertText", "shouldAlert", "status", "update"]],
      ["open_update_release", { url: "https://github.com/x" }, ["current", "previous"]],
      ["open_repository", {}, ["current", "previous"]],
      ["register_shutdown", { input: { kind: "once", date: "2026-07-12", time: "23:30", days: [] } }, ["shutdown", "status"]],
      ["cancel_shutdown", { kind: "once" }, ["shutdown", "status"]],
      ["get_app_state", {}, ["appVersion", "control", "monitor", "settings", "status", "update"]],
    ];
    for (const [command, args, keys] of cases) {
      const preview = createBrowserPreview();
      expect(Object.keys(preview(command, args)).sort(), command).toEqual([...keys].sort());
    }
  });
});
