// @vitest-environment jsdom
import React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  App,
  EMPTY_STATE,
  GlassDatePicker,
  GlassSelect,
  ScheduleTab,
  SettingsTab,
} from "./main.jsx";

beforeEach(() => {
  window.requestAnimationFrame = (callback) => window.setTimeout(callback, 0);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.__TAURI_INTERNALS__;
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("키보드 접근성", () => {
  test("GlassSelect는 화살표로 이동해 선택하고 trigger로 focus를 돌려준다", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <GlassSelect
        value="normal"
        onChange={onChange}
        options={[
          { value: "normal", label: "일반" },
          { value: "high", label: "고성능" },
          { value: "low", label: "저전력" },
        ]}
      />,
    );

    const trigger = screen.getByRole("button", { name: "일반" });
    trigger.focus();
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");

    expect(onChange).toHaveBeenCalledWith("high");
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  test("GlassDatePicker는 날짜 grid를 화살표로 이동해 선택한다", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<GlassDatePicker value="2026-07-11" onChange={onChange} />);

    const trigger = screen.getByRole("button", { name: /2026-07-11/ });
    await user.click(trigger);
    expect(document.activeElement?.textContent).toBe("11");
    await user.keyboard("{ArrowRight}{Enter}");

    expect(onChange).toHaveBeenCalledWith("2026-07-12");
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  test("GlassSelect와 GlassDatePicker는 Tab 한 번으로 trigger 다음 요소로 이동한다", async () => {
    const user = userEvent.setup();
    const { unmount } = render(
      <>
        <button type="button">이전</button>
        <GlassSelect value="normal" onChange={vi.fn()} options={["normal", "high"]} />
        <button type="button">다음</button>
      </>,
    );
    await user.click(screen.getByRole("button", { name: "normal" }));
    await user.keyboard("{Tab}");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "다음" }));
    await user.click(screen.getByRole("button", { name: "normal" }));
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "이전" }));

    unmount();
    render(
      <>
        <button type="button">이전</button>
        <GlassDatePicker value="2026-07-11" onChange={vi.fn()} />
        <button type="button">다음</button>
      </>,
    );
    await user.click(screen.getByRole("button", { name: /2026-07-11/ }));
    await user.keyboard("{Tab}");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "다음" }));
    await user.click(screen.getByRole("button", { name: /2026-07-11/ }));
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "이전" }));
  });

  test("닫힌 accordion은 inert이며 열면 다시 조작 가능하다", async () => {
    const user = userEvent.setup();
    render(<ScheduleTab state={EMPTY_STATE} pending={null} runCommand={vi.fn()} />);

    const header = screen.getByRole("button", { name: /자동 모드 전환/ });
    const panel = document.getElementById("schedule-automation-panel");
    expect(panel.getAttribute("aria-hidden")).toBe("true");
    expect(panel.hasAttribute("inert")).toBe(true);

    await user.click(header);
    expect(panel.getAttribute("aria-hidden")).toBe("false");
    expect(panel.hasAttribute("inert")).toBe(false);
  });

  test("단발과 매주 예약 시간은 동시에 활성화돼도 서로 덮어쓰지 않는다", async () => {
    const user = userEvent.setup();
    const runCommand = vi.fn();
    const state = {
      ...EMPTY_STATE,
      shutdown: {
        ...EMPTY_STATE.shutdown,
        onceActive: true,
        onceDate: "2026-07-12",
        onceTime: "21:10",
        weeklyActive: true,
        weeklyDays: ["MON", "FRI"],
        weeklyTime: "06:45",
      },
    };
    render(<ScheduleTab state={state} pending={null} runCommand={runCommand} />);
    await waitFor(() =>
      expect([...document.querySelectorAll(".stepper .num")].map((node) => node.textContent)).toEqual([
        "21",
        "10",
      ]),
    );

    await user.click(screen.getByRole("button", { name: "매주 반복" }));
    expect([...document.querySelectorAll(".stepper .num")].map((node) => node.textContent)).toEqual([
      "06",
      "45",
    ]);
    await user.click(screen.getByRole("button", { name: /예약 등록/ }));
    expect(runCommand).toHaveBeenLastCalledWith(
      "shutdown-register",
      "register_shutdown",
      expect.objectContaining({ input: expect.objectContaining({ kind: "weekly", time: "06:45" }) }),
    );
  });

  test("accent 선택은 저장 command 인자만 보내고 성공 payload 전에는 낙관 변경하지 않는다", async () => {
    const user = userEvent.setup();
    const runCommand = vi.fn();
    render(
      <SettingsTab
        state={EMPTY_STATE}
        pending={null}
        runCommand={runCommand}
        showToast={vi.fn()}
      />,
    );
    const teal = screen.getByRole("button", { name: "청록" });
    const gold = screen.getByRole("button", { name: "골드" });
    await user.click(gold);
    expect(runCommand).toHaveBeenCalledWith("setting-accent_palette", "set_setting", {
      input: expect.objectContaining({ key: "accent_palette", intValue: 1 }),
    });
    expect(teal.getAttribute("aria-pressed")).toBe("true");
    expect(gold.getAttribute("aria-pressed")).toBe("false");
  });

  test("설정 저장 중 기본 모드와 모니터 주기 select는 추가 입력을 받지 않는다", async () => {
    const user = userEvent.setup();
    const runCommand = vi.fn();
    render(
      <SettingsTab
        state={EMPTY_STATE}
        pending="setting-theme_mode"
        runCommand={runCommand}
        showToast={vi.fn()}
      />,
    );

    const defaultMode = screen.getByRole("button", { name: "없음" });
    const monitorInterval = screen.getByRole("button", { name: "1초" });
    expect(defaultMode.disabled).toBe(true);
    expect(monitorInterval.disabled).toBe(true);

    await user.click(defaultMode);
    await user.click(monitorInterval);
    expect(runCommand).not.toHaveBeenCalled();
  });

  test("tablist는 End와 Ctrl+숫자 이동 및 roving tab stop을 지원한다", async () => {
    const user = userEvent.setup();
    render(<App />);
    const tabs = screen.getAllByRole("tab");

    tabs[0].focus();
    await user.keyboard("{End}");
    expect(tabs[3].getAttribute("aria-selected")).toBe("true");
    expect(tabs[3].tabIndex).toBe(0);
    expect(tabs[0].tabIndex).toBe(-1);

    await user.keyboard("{Control>}2{/Control}");
    expect(tabs[1].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tabs[1]);
    expect(document.getElementById("app-tabpanel").getAttribute("aria-labelledby")).toBe(
      "app-tab-1",
    );
    expect(tabs.every((tab) => document.getElementById(tab.getAttribute("aria-controls")))).toBe(
      true,
    );
  });

  test("사용자 명령 중 app surface와 live status가 진행 상태를 알린다", async () => {
    let blockRefresh = false;
    let finishRefresh;
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(EMPTY_STATE);
      if (command === "list_schedule_rules") return Promise.resolve(EMPTY_STATE.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(EMPTY_STATE.shutdown);
      if (command === "refresh_game_status" && blockRefresh) {
        return new Promise((resolve) => {
          finishRefresh = resolve;
        });
      }
      return Promise.resolve({
        status: { current: "상태 확인 완료.", previous: "" },
        control: EMPTY_STATE.control,
      });
    });
    render(<App nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);
    const content = document.querySelector(".content");
    await waitFor(() => expect(content.getAttribute("aria-busy")).toBe("false"));

    const refresh = screen.getByRole("button", { name: /상태 새로고침/ });
    blockRefresh = true;
    fireEvent.click(refresh);
    expect(content.getAttribute("aria-busy")).toBe("true");
    expect(document.querySelector(".statusbar").textContent).toContain("게임 상태 확인 중");
    finishRefresh({
      status: { current: "상태 확인 완료.", previous: "" },
      control: EMPTY_STATE.control,
    });
    await waitFor(() => expect(content.getAttribute("aria-busy")).toBe("false"));
  });

  test("업데이트 알림 timer는 설정이 켜진 경우에만 시작한다", async () => {
    vi.useFakeTimers();
    const enabledState = {
      ...EMPTY_STATE,
      settings: { ...EMPTY_STATE.settings, updateAlertEnabled: true, updateCheckIntervalMs: 21600000 },
    };
    const invokeEnabled = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(enabledState);
      if (command === "list_schedule_rules") return Promise.resolve(enabledState.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(enabledState.shutdown);
      if (command === "check_update_alert") {
        return Promise.resolve({
          status: { current: "최신 버전입니다.", previous: "" },
          update: enabledState.update,
          shouldAlert: false,
          alertText: "",
        });
      }
      return Promise.resolve({
        status: { current: "상태 확인 완료.", previous: "" },
        control: enabledState.control,
      });
    });
    render(<App initialState={enabledState} nativeInvoke={invokeEnabled} runtimeCheck={() => true} />);
    await act(async () => {});
    expect(invokeEnabled).not.toHaveBeenCalledWith("check_update_alert");
    await act(async () => vi.advanceTimersByTimeAsync(8000));
    expect(invokeEnabled).toHaveBeenCalledWith("check_update_alert");

    cleanup();
    const disabledState = {
      ...enabledState,
      settings: { ...enabledState.settings, updateAlertEnabled: false },
    };
    const invokeDisabled = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(disabledState);
      if (command === "list_schedule_rules") return Promise.resolve(disabledState.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(disabledState.shutdown);
      return Promise.resolve({
        status: { current: "상태 확인 완료.", previous: "" },
        control: disabledState.control,
      });
    });
    render(<App initialState={disabledState} nativeInvoke={invokeDisabled} runtimeCheck={() => true} />);
    await act(async () => vi.advanceTimersByTimeAsync(21600000));
    expect(invokeDisabled).not.toHaveBeenCalledWith("check_update_alert");
    vi.useRealTimers();
  });

  test("느린 자동 업데이트 확인은 최신 수동 확인 결과와 toast를 덮어쓰지 않는다", async () => {
    vi.useFakeTimers();
    const automaticUpdate = deferred();
    const enabledState = {
      ...EMPTY_STATE,
      settings: {
        ...EMPTY_STATE.settings,
        updateAlertEnabled: true,
        updateCheckIntervalMs: 21600000,
      },
    };
    const manualUpdate = {
      ...EMPTY_STATE.update,
      statusText: "수동 확인 최신 결과",
      latestVersion: "0.2.0",
    };
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(enabledState);
      if (command === "list_schedule_rules") return Promise.resolve(enabledState.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(enabledState.shutdown);
      if (command === "check_update_alert") return automaticUpdate.promise;
      if (command === "check_for_updates") {
        return Promise.resolve({
          status: { current: "수동 확인 완료", previous: "" },
          update: manualUpdate,
        });
      }
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    render(<App initialState={enabledState} nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);

    await act(async () => vi.advanceTimersByTimeAsync(8000));
    vi.useRealTimers();
    expect(nativeInvoke).toHaveBeenCalledWith("check_update_alert");
    fireEvent.click(screen.getByRole("tab", { name: "설정" }));
    fireEvent.click(screen.getByRole("button", { name: "업데이트 확인" }));
    await waitFor(() => expect(screen.getByText("수동 확인 최신 결과")).toBeTruthy());

    await act(async () =>
      automaticUpdate.resolve({
        status: { current: "오래된 자동 확인", previous: "" },
        update: { ...EMPTY_STATE.update, statusText: "오래된 자동 결과" },
        shouldAlert: true,
        alertText: "오래된 자동 알림",
      }),
    );
    expect(screen.getByText("수동 확인 최신 결과")).toBeTruthy();
    expect(screen.queryByText("오래된 자동 결과")).toBeNull();
    expect(screen.queryByText("오래된 자동 알림")).toBeNull();
  });

  test("모니터 탭 이탈 후 늦게 도착한 snapshot은 재진입 화면에 적용하지 않는다", async () => {
    const staleSnapshot = deferred();
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(EMPTY_STATE);
      if (command === "list_schedule_rules") return Promise.resolve(EMPTY_STATE.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(EMPTY_STATE.shutdown);
      if (command === "get_monitor_snapshot") return staleSnapshot.promise;
      if (command === "stop_monitor_session") return Promise.resolve(null);
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    render(<App nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);

    fireEvent.click(screen.getByRole("tab", { name: "모니터" }));
    await waitFor(() => expect(nativeInvoke).toHaveBeenCalledWith("get_monitor_snapshot", undefined));
    fireEvent.click(screen.getByRole("tab", { name: "제어" }));
    fireEvent.click(screen.getByRole("tab", { name: "모니터" }));

    await act(async () =>
      staleSnapshot.resolve({
        ...EMPTY_STATE.monitor,
        running: true,
        statusText: "이전 모니터 세션의 오래된 snapshot",
      }),
    );
    expect(screen.queryByText("이전 모니터 세션의 오래된 snapshot")).toBeNull();
    expect(screen.getByText(EMPTY_STATE.monitor.statusText)).toBeTruthy();
  });

  test("느린 settings 조회는 최신 accent 저장 결과를 덮어쓰지 않는다", async () => {
    const staleSettings = deferred();
    const savedSettings = { ...EMPTY_STATE.settings, accentPalette: 1 };
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(EMPTY_STATE);
      if (command === "list_schedule_rules") return Promise.resolve(EMPTY_STATE.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(EMPTY_STATE.shutdown);
      if (command === "get_settings") return staleSettings.promise;
      if (command === "set_setting") {
        return Promise.resolve({
          status: { current: "액센트 색상을 저장했습니다.", previous: "" },
          settings: savedSettings,
        });
      }
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    const user = userEvent.setup();
    render(<App nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);

    await user.click(screen.getByRole("tab", { name: "설정" }));
    await waitFor(() => expect(nativeInvoke).toHaveBeenCalledWith("get_settings", undefined));
    await user.click(screen.getByRole("button", { name: "골드" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "골드" }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );

    await act(async () => staleSettings.resolve(EMPTY_STATE.settings));
    expect(screen.getByRole("button", { name: "골드" }).getAttribute("aria-pressed")).toBe("true");
  });

  test("느린 schedule 조회는 최신 규칙 추가 결과를 덮어쓰지 않는다", async () => {
    const staleSchedule = deferred();
    const newRule = {
      id: 1,
      name: "새 규칙",
      kind: "daily",
      date: null,
      startTime: "19:00",
      endTime: "23:00",
      mode: "high",
      active: true,
      summary: "매일 19:00~23:00 · 고성능",
    };
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(EMPTY_STATE);
      if (command === "list_schedule_rules") return staleSchedule.promise;
      if (command === "get_shutdown_state") return Promise.resolve(EMPTY_STATE.shutdown);
      if (command === "add_schedule_rule") {
        return Promise.resolve({
          status: { current: "스케줄 규칙이 추가되었습니다.", previous: "" },
          schedule: { activeRuleInfo: "활성 규칙 없음.", rules: [newRule], empty: false },
        });
      }
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    const user = userEvent.setup();
    render(<App nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);
    await user.click(screen.getByRole("tab", { name: "스케줄" }));
    await user.click(screen.getByRole("button", { name: /자동 모드 전환/ }));
    await user.type(screen.getByPlaceholderText(/규칙 이름/), "새 규칙");
    await user.click(screen.getByRole("button", { name: /규칙 추가/ }));
    await waitFor(() => expect(screen.getByText("새 규칙")).toBeTruthy());

    await act(async () => staleSchedule.resolve(EMPTY_STATE.schedule));
    expect(screen.getByText("새 규칙")).toBeTruthy();
  });

  test("느린 shutdown 조회는 최신 취소 결과를 덮어쓰지 않는다", async () => {
    const staleShutdown = deferred();
    const activeState = {
      ...EMPTY_STATE,
      shutdown: {
        ...EMPTY_STATE.shutdown,
        onceActive: true,
        onceText: "2026-07-12 23:30",
        onceDate: "2026-07-12",
        onceTime: "23:30",
      },
    };
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(activeState);
      if (command === "list_schedule_rules") return Promise.resolve(activeState.schedule);
      if (command === "get_shutdown_state") return staleShutdown.promise;
      if (command === "cancel_shutdown") {
        return Promise.resolve({
          status: { current: "단발 예약을 취소했습니다.", previous: "" },
          shutdown: EMPTY_STATE.shutdown,
        });
      }
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    const user = userEvent.setup();
    render(<App initialState={activeState} nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);
    await user.click(screen.getByRole("tab", { name: "스케줄" }));
    await user.click(screen.getByRole("button", { name: "단발 취소" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "단발 취소" })).toBeNull());

    await act(async () => staleShutdown.resolve(activeState.shutdown));
    expect(screen.queryByRole("button", { name: "단발 취소" })).toBeNull();
  });

  test("스케줄 탭의 15초 polling은 schedule과 shutdown을 함께 갱신한다", async () => {
    vi.useFakeTimers();
    const nativeInvoke = vi.fn((command) => {
      if (command === "get_app_state") return Promise.resolve(EMPTY_STATE);
      if (command === "list_schedule_rules") return Promise.resolve(EMPTY_STATE.schedule);
      if (command === "get_shutdown_state") return Promise.resolve(EMPTY_STATE.shutdown);
      return Promise.resolve({ status: EMPTY_STATE.status, control: EMPTY_STATE.control });
    });
    render(<App nativeInvoke={nativeInvoke} runtimeCheck={() => true} />);
    fireEvent.click(screen.getByRole("tab", { name: "스케줄" }));
    await act(async () => {});
    nativeInvoke.mockClear();

    await act(async () => vi.advanceTimersByTimeAsync(15_000));
    expect(nativeInvoke).toHaveBeenCalledWith("list_schedule_rules", undefined);
    expect(nativeInvoke).toHaveBeenCalledWith("get_shutdown_state", undefined);
    vi.useRealTimers();
  });
});
