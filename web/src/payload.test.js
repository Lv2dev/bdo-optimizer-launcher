import { describe, expect, it } from "vitest";
import { normalizePayload } from "./payload.js";

describe("normalizePayload", () => {
  it.each(["open_log_folder", "open_update_release", "open_repository"])(
    "%s wraps a direct StatusDto response",
    (command) => {
      const status = { current: "완료", previous: "" };

      expect(normalizePayload(command, status)).toEqual({ status });
    },
  );

  it("keeps wrapped command payloads unchanged", () => {
    const payload = {
      status: { current: "완료", previous: "" },
      control: { gameRunning: false },
    };

    expect(normalizePayload("refresh_game_status", payload)).toBe(payload);
  });

  it.each([
    ["list_schedule_rules", "schedule"],
    ["get_shutdown_state", "shutdown"],
    ["get_settings", "settings"],
    ["get_monitor_snapshot", "monitor"],
  ])("%s wraps its direct state DTO", (command, field) => {
    const direct = { marker: command };
    expect(normalizePayload(command, direct)).toEqual({ [field]: direct });
  });
});
