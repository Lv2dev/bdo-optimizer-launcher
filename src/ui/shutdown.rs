use crate::backend::shutdown::{self, WeeklyInfo};
use crate::ui::push_status;
use crate::AppWindow;
use chrono::{DateTime, Duration, Local};
use slint::ComponentHandle;

pub fn apply_initial(app: &AppWindow) {
    refresh_shutdown_ui(app);
}

/// schtasks 조회 + UI property 4종 동기화. status_timer/콜백 양쪽에서 호출.
/// M81 (Mj2): schtasks /query /xml 2회는 100~500ms 소요되므로 메인 UI 스레드에서 호출하면
/// 5초마다 UI freeze. 백그라운드 thread에서 호출 + 시간 차이 계산까지 완료 후
/// `slint::invoke_from_event_loop`로 결과 string만 메인 스레드에 push해 stall 제거.
pub fn refresh_shutdown_ui(app: &AppWindow) {
    let app_weak = app.as_weak();
    std::thread::spawn(move || {
        let snap = shutdown::query_schedules();
        let now = Local::now();

        let (once_text, once_active) = match snap.once {
            Some(dt) => (
                format!("{} ({})", fmt_absolute(dt), fmt_remaining(dt, now)),
                true,
            ),
            None => (String::new(), false),
        };
        let (weekly_text, weekly_active) = match snap.weekly {
            Some(info) => (fmt_weekly(&info, now), true),
            None => (String::new(), false),
        };

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app_weak.upgrade() {
                app.set_shutdown_once_text(once_text.into());
                app.set_shutdown_once_active(once_active);
                app.set_shutdown_weekly_text(weekly_text.into());
                app.set_shutdown_weekly_active(weekly_active);
            }
        });
    });
}

pub fn register(app: &AppWindow) {
    app.on_register_shutdown({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let weekly = app.get_shutdown_weekly();
                let time = app.get_shutdown_time().to_string();
                let msg = if weekly {
                    let mut days: Vec<&'static str> = Vec::new();
                    if app.get_shutdown_mon() {
                        days.push("MON");
                    }
                    if app.get_shutdown_tue() {
                        days.push("TUE");
                    }
                    if app.get_shutdown_wed() {
                        days.push("WED");
                    }
                    if app.get_shutdown_thu() {
                        days.push("THU");
                    }
                    if app.get_shutdown_fri() {
                        days.push("FRI");
                    }
                    if app.get_shutdown_sat() {
                        days.push("SAT");
                    }
                    if app.get_shutdown_sun() {
                        days.push("SUN");
                    }
                    match shutdown::register_weekly_shutdown(&days, &time) {
                        Ok(()) => {
                            tracing::info!(days = ?days, time = %time, "weekly shutdown registered");
                            format!("매주 반복 종료 예약 등록 완료 ({}시).", time)
                        }
                        Err(e) => {
                            tracing::error!(days = ?days, time = %time, error = %e, "weekly shutdown register failed");
                            format!("오류: {e}")
                        }
                    }
                } else {
                    let date = app.get_shutdown_once_date().to_string();
                    match shutdown::register_once_shutdown(&date, &time) {
                        Ok(()) => {
                            tracing::info!(date = %date, time = %time, "once shutdown registered");
                            format!("단발 종료 예약 등록 완료: {} {}.", date, time)
                        }
                        Err(e) => {
                            tracing::error!(date = %date, time = %time, error = %e, "once shutdown register failed");
                            format!("오류: {e}")
                        }
                    }
                };
                push_status(&app, msg);
                refresh_shutdown_ui(&app);
            }
        }
    });

    app.on_cancel_once_shutdown({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let msg = match shutdown::cancel_once() {
                    Ok(()) => "단발 종료 예약이 취소되었습니다.".to_string(),
                    Err(e) => format!("오류: {e}"),
                };
                push_status(&app, msg);
                refresh_shutdown_ui(&app);
            }
        }
    });

    app.on_cancel_weekly_shutdown({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let msg = match shutdown::cancel_weekly() {
                    Ok(()) => "매주 반복 종료 예약이 취소되었습니다.".to_string(),
                    Err(e) => format!("오류: {e}"),
                };
                push_status(&app, msg);
                refresh_shutdown_ui(&app);
            }
        }
    });
}

fn fmt_absolute(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

fn fmt_remaining(target: DateTime<Local>, now: DateTime<Local>) -> String {
    fmt_remaining_from_duration(target - now)
}

fn fmt_remaining_from_duration(d: Duration) -> String {
    let secs = d.num_seconds();
    if secs < 60 {
        return "곧 실행".to_string();
    }
    let total_min = secs / 60;
    if total_min < 60 {
        return format!("{}분 남음", total_min);
    }
    let total_hours = total_min / 60;
    let mins = total_min % 60;
    if total_hours < 24 {
        if mins == 0 {
            return format!("{}시간 남음", total_hours);
        }
        return format!("{}시간 {}분 남음", total_hours, mins);
    }
    let days = total_hours / 24;
    let hours = total_hours % 24;
    if hours == 0 {
        format!("{}일 남음", days)
    } else {
        format!("{}일 {}시간 남음", days, hours)
    }
}

fn fmt_weekly(info: &WeeklyInfo, now: DateTime<Local>) -> String {
    let days_kr = fmt_weekly_days(&info.days);
    let (h, m) = info.time_hm;
    format!(
        "매주 {} {:02}:{:02} (다음 {})",
        days_kr,
        h,
        m,
        fmt_remaining(info.next_run, now)
    )
}

fn fmt_weekly_days(days: &[&str]) -> String {
    days.iter()
        .map(|d| match *d {
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

#[cfg(test)]
mod tests {
    use super::fmt_remaining_from_duration;
    use chrono::Duration;

    #[test]
    fn fmt_zero_or_negative() {
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(-1)),
            "곧 실행"
        );
        assert_eq!(fmt_remaining_from_duration(Duration::seconds(0)), "곧 실행");
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(30)),
            "곧 실행"
        );
    }

    #[test]
    fn fmt_minutes_only() {
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(60)),
            "1분 남음"
        );
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(59 * 60)),
            "59분 남음"
        );
    }

    #[test]
    fn fmt_hours_minutes() {
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(3600)),
            "1시간 남음"
        );
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(3600 + 23 * 60)),
            "1시간 23분 남음"
        );
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(23 * 3600 + 59 * 60)),
            "23시간 59분 남음"
        );
    }

    #[test]
    fn fmt_days_hours() {
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(24 * 3600)),
            "1일 남음"
        );
        assert_eq!(
            fmt_remaining_from_duration(Duration::seconds(2 * 24 * 3600 + 5 * 3600)),
            "2일 5시간 남음"
        );
    }
}
