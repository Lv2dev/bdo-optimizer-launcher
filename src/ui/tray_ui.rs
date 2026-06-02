use crate::backend::{schedule, shutdown, tray};
use crate::ui::{control, push_status, AppState};
use crate::AppWindow;
use slint::ComponentHandle;
use std::rc::Rc;

#[derive(Clone, Copy)]
pub enum TrayCommand {
    ToggleWindow,
    ShowWindow,
    ApplyHigh,
    ApplyNormal,
    ApplyLowPower,
    CancelShutdown,
    Quit,
}

pub fn dispatch(app: &AppWindow, state: &AppState, cmd: TrayCommand, handle: &tray::TrayHandle) {
    match cmd {
        TrayCommand::ToggleWindow => {
            let visible = app.window().is_visible();
            if visible {
                let _ = app.window().hide();
                tray::set_toggle_label(handle, false);
            } else {
                let _ = app.window().show();
                tray::set_toggle_label(handle, true);
            }
        }
        TrayCommand::ShowWindow => {
            let _ = app.window().show();
            tray::set_toggle_label(handle, true);
        }
        TrayCommand::ApplyHigh => {
            control::apply_user_mode(app, state, schedule::OptimizeMode::High);
        }
        TrayCommand::ApplyNormal => {
            control::apply_user_mode(app, state, schedule::OptimizeMode::Normal);
        }
        TrayCommand::ApplyLowPower => {
            control::apply_user_mode(app, state, schedule::OptimizeMode::LowPower);
        }
        TrayCommand::CancelShutdown => {
            // 트레이 "예약 취소"는 단발만 (spec M46: 메뉴 비대 회피, 매주는 GUI에서만).
            let msg = match shutdown::cancel_once() {
                Ok(()) => "단발 종료 예약이 취소되었습니다.".to_string(),
                Err(e) => format!("오류: {e}"),
            };
            push_status(app, msg);
            crate::ui::shutdown::refresh_shutdown_ui(app);
        }
        TrayCommand::Quit => {
            slint::quit_event_loop().ok();
        }
    }
}

// 트레이 init + 워커 thread + tray_timer + close-requested를 일괄 처리.
// 반환: (트레이 핸들 옵션, tray_timer). 핸들은 main에서 이벤트 루프 분기 결정에 사용.
pub fn register(app: &AppWindow, state: &AppState) -> (Option<Rc<tray::TrayHandle>>, slint::Timer) {
    let timer = slint::Timer::default();
    let tray_handle: Option<Rc<tray::TrayHandle>> = match tray::build() {
        Ok(h) => Some(Rc::new(h)),
        Err(e) => {
            tracing::warn!(error = %e, "tray init failed, falling back to last-window-close");
            None
        }
    };
    if let Some(handle) = tray_handle.as_ref() {
        *state.mode.tray_handle.borrow_mut() = Some(Rc::clone(handle));
        tray::set_mode_indicator(handle.as_ref(), None);
    }

    let tray_cmd_rx: Option<std::sync::mpsc::Receiver<TrayCommand>> =
        if let Some(ref handle) = tray_handle {
            let (tx, rx) = std::sync::mpsc::channel::<TrayCommand>();
            let id_toggle = handle.menu_id_toggle.clone();
            let id_high = handle.menu_id_high.clone();
            let id_normal = handle.menu_id_normal.clone();
            let id_low_power = handle.menu_id_low_power.clone();
            let id_cancel = handle.menu_id_cancel_shutdown.clone();
            let id_quit = handle.menu_id_quit.clone();
            std::thread::spawn(move || {
                let menu_rx = tray_icon::menu::MenuEvent::receiver();
                let tray_rx = tray_icon::TrayIconEvent::receiver();
                // M80: 두 receiver를 crossbeam_channel::select!로 blocking 대기.
                // 기존 50ms busy-poll(std::thread::sleep) 제거 → worker thread는 OS-level
                // 이벤트 대기 상태로 영구 wake 부담 없음(노트북 modern standby / E-core idle
                // 진입 가능). 채널이 닫히면 send 실패로 자연 종료.
                loop {
                    crossbeam_channel::select! {
                        recv(menu_rx) -> ev => {
                            let ev = match ev {
                                Ok(e) => e,
                                Err(_) => break,
                            };
                            let id = ev.id.0.clone();
                            let cmd = if id == id_toggle {
                                Some(TrayCommand::ToggleWindow)
                            } else if id == id_high {
                                Some(TrayCommand::ApplyHigh)
                            } else if id == id_normal {
                                Some(TrayCommand::ApplyNormal)
                            } else if id == id_low_power {
                                Some(TrayCommand::ApplyLowPower)
                            } else if id == id_cancel {
                                Some(TrayCommand::CancelShutdown)
                            } else if id == id_quit {
                                Some(TrayCommand::Quit)
                            } else {
                                None
                            };
                            if let Some(cmd) = cmd {
                                let _ = tx.send(cmd);
                            }
                        }
                        recv(tray_rx) -> ev => {
                            let ev = match ev {
                                Ok(e) => e,
                                Err(_) => break,
                            };
                            if let tray_icon::TrayIconEvent::DoubleClick { .. } = ev {
                                let _ = tx.send(TrayCommand::ShowWindow);
                            }
                        }
                    }
                }
            });
            Some(rx)
        } else {
            None
        };

    if let (Some(rx), Some(handle)) = (tray_cmd_rx, tray_handle.as_ref()) {
        let app_weak = app.as_weak();
        let handle_rc = Rc::clone(handle);
        let state = state.clone();
        // M80: 50ms → 100ms. main thread wake 빈도 절반화 (사용자 체감 latency 50→100ms는
        // 인간 reaction time(200ms+) 밑이라 무감각). worker thread는 이미 blocking select!라
        // 이 Timer가 유일한 main wake 소스.
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(100),
            move || {
                if let Some(app) = app_weak.upgrade() {
                    while let Ok(cmd) = rx.try_recv() {
                        dispatch(&app, &state, cmd, handle_rc.as_ref());
                    }
                }
            },
        );
    }

    if let Some(handle_for_close) = tray_handle.as_ref().map(Rc::clone) {
        let app_weak = app.as_weak();
        app.window().on_close_requested(move || {
            // 설정 토글에 따라 트레이로 숨기기 vs 완전 종료 분기.
            let to_tray = app_weak
                .upgrade()
                .map(|a| a.get_close_to_tray())
                .unwrap_or(true);
            if to_tray {
                tray::set_toggle_label(&handle_for_close, false);
                slint::CloseRequestResponse::HideWindow
            } else {
                slint::quit_event_loop().ok();
                slint::CloseRequestResponse::HideWindow
            }
        });
    }

    (tray_handle, timer)
}
