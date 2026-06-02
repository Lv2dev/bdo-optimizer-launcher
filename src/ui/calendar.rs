use crate::AppWindow;
use chrono::{Datelike, Local, NaiveDate};
use slint::{ComponentHandle, ModelRc, VecModel};

pub fn weekday_ko(w: chrono::Weekday) -> &'static str {
    match w {
        chrono::Weekday::Sun => "일",
        chrono::Weekday::Mon => "월",
        chrono::Weekday::Tue => "화",
        chrono::Weekday::Wed => "수",
        chrono::Weekday::Thu => "목",
        chrono::Weekday::Fri => "금",
        chrono::Weekday::Sat => "토",
    }
}

pub fn build_cal_days(year: i32, month: u32) -> ModelRc<crate::CalendarDayUi> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let offset = first.weekday().num_days_from_sunday() as usize;
    let next_first = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap_or_default();
    let days_in = next_first.signed_duration_since(first).num_days() as usize;

    let cells: Vec<crate::CalendarDayUi> = (0..42)
        .map(|i| {
            if i < offset || i >= offset + days_in {
                crate::CalendarDayUi {
                    day: 0,
                    label: "".into(),
                }
            } else {
                let d = (i - offset + 1) as i32;
                crate::CalendarDayUi {
                    day: d,
                    label: format!("{d}").into(),
                }
            }
        })
        .collect();
    ModelRc::new(VecModel::from(cells))
}

pub fn cal_month_label(year: i32, month: u32) -> String {
    format!("{year}년 {month}월")
}

pub fn cal_selected_label(year: i32, month: u32, day: u32) -> String {
    let date = NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default();
    let dow = weekday_ko(date.weekday());
    format!("{year:04}-{month:02}-{day:02} ({dow})")
}

pub fn apply_initial(app: &AppWindow) {
    let today = Local::now().date_naive();
    // M51: 단발 종료 기본값은 다음날. 캘린더 표시도 내일이 속한 월/연으로 자동 점프
    // (월말 케이스 대응). cal_today_day는 캘린더가 보여주는 월에 오늘이 있을 때만 강조.
    let tomorrow = today + chrono::Duration::days(1);
    let ty = tomorrow.year();
    let tm = tomorrow.month();
    let td = tomorrow.day();
    app.set_cal_year(ty);
    app.set_cal_month(tm as i32);
    app.set_cal_today_day(if today.year() == ty && today.month() == tm {
        today.day() as i32
    } else {
        0
    });
    app.set_cal_selected_day(td as i32);
    app.set_cal_days(build_cal_days(ty, tm));
    app.set_cal_month_label(cal_month_label(ty, tm).into());
    app.set_cal_selected_label(cal_selected_label(ty, tm, td).into());
    app.set_shutdown_once_date(format!("{ty:04}-{tm:02}-{td:02}").into());
    app.set_shutdown_hour(0);
    app.set_shutdown_minute(0);
    app.set_shutdown_hour_text("00".into());
    app.set_shutdown_minute_text("00".into());
    app.set_shutdown_time("00:00".into());
}

pub fn register(app: &AppWindow) {
    app.on_cal_prev_month({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let mut year = app.get_cal_year();
                let mut month = app.get_cal_month() as u32;
                if month == 1 {
                    if year <= 1 {
                        return;
                    }
                    year -= 1;
                    month = 12;
                } else {
                    month -= 1;
                }
                app.set_cal_year(year);
                app.set_cal_month(month as i32);
                app.set_cal_days(build_cal_days(year, month));
                app.set_cal_month_label(cal_month_label(year, month).into());
                let today = Local::now().date_naive();
                let td = if today.year() == year && today.month() == month {
                    today.day() as i32
                } else {
                    0
                };
                app.set_cal_today_day(td);
            }
        }
    });

    app.on_cal_next_month({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let mut year = app.get_cal_year();
                let mut month = app.get_cal_month() as u32;
                if month == 12 {
                    year += 1;
                    month = 1;
                } else {
                    month += 1;
                }
                app.set_cal_year(year);
                app.set_cal_month(month as i32);
                app.set_cal_days(build_cal_days(year, month));
                app.set_cal_month_label(cal_month_label(year, month).into());
                let today = Local::now().date_naive();
                let td = if today.year() == year && today.month() == month {
                    today.day() as i32
                } else {
                    0
                };
                app.set_cal_today_day(td);
            }
        }
    });

    app.on_cal_select_day({
        let app = app.as_weak();
        move |day| {
            if let Some(app) = app.upgrade() {
                let year = app.get_cal_year();
                let month = app.get_cal_month() as u32;
                let d = day as u32;
                app.set_cal_selected_day(day);
                app.set_cal_selected_label(cal_selected_label(year, month, d).into());
                app.set_shutdown_once_date(format!("{year:04}-{month:02}-{d:02}").into());
            }
        }
    });

    app.on_shutdown_hour_up({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let h = (app.get_shutdown_hour() + 1) % 24;
                let m = app.get_shutdown_minute();
                app.set_shutdown_hour(h);
                app.set_shutdown_hour_text(format!("{h:02}").into());
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });

    app.on_shutdown_hour_down({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let h = (app.get_shutdown_hour() - 1 + 24) % 24;
                let m = app.get_shutdown_minute();
                app.set_shutdown_hour(h);
                app.set_shutdown_hour_text(format!("{h:02}").into());
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });

    app.on_shutdown_minute_up({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let h = app.get_shutdown_hour();
                let m = (app.get_shutdown_minute() + 1) % 60;
                app.set_shutdown_minute(m);
                app.set_shutdown_minute_text(format!("{m:02}").into());
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });

    app.on_shutdown_minute_down({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let h = app.get_shutdown_hour();
                let m = (app.get_shutdown_minute() - 1 + 60) % 60;
                app.set_shutdown_minute(m);
                app.set_shutdown_minute_text(format!("{m:02}").into());
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });

    app.on_shutdown_hour_set({
        let app = app.as_weak();
        move |text| {
            if let Some(app) = app.upgrade() {
                let trimmed = text.trim();
                let h = if trimmed.is_empty() {
                    app.get_shutdown_hour()
                } else {
                    match trimmed.parse::<i64>() {
                        Ok(v) => v.clamp(0, 23) as i32,
                        Err(_) => app.get_shutdown_hour(),
                    }
                };
                let m = app.get_shutdown_minute();
                app.set_shutdown_hour(h);
                let formatted: slint::SharedString = format!("{h:02}").into();
                if app.get_shutdown_hour_text() == formatted {
                    app.set_shutdown_hour_text("".into());
                }
                app.set_shutdown_hour_text(formatted);
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });

    app.on_shutdown_minute_set({
        let app = app.as_weak();
        move |text| {
            if let Some(app) = app.upgrade() {
                let trimmed = text.trim();
                let m = if trimmed.is_empty() {
                    app.get_shutdown_minute()
                } else {
                    match trimmed.parse::<i64>() {
                        Ok(v) => v.clamp(0, 59) as i32,
                        Err(_) => app.get_shutdown_minute(),
                    }
                };
                let h = app.get_shutdown_hour();
                app.set_shutdown_minute(m);
                let formatted: slint::SharedString = format!("{m:02}").into();
                if app.get_shutdown_minute_text() == formatted {
                    app.set_shutdown_minute_text("".into());
                }
                app.set_shutdown_minute_text(formatted);
                app.set_shutdown_time(format!("{h:02}:{m:02}").into());
            }
        }
    });
}
