use crate::backend::schedule;
use crate::ui::{control, push_status, AppState};
use crate::{AppWindow, OptimizeMode as UiOptimizeMode, RuleKind as UiRuleKind, ScheduleRuleUi};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;

fn ui_kind_to_backend(k: UiRuleKind, date: &str) -> schedule::ScheduleKind {
    match k {
        UiRuleKind::Daily => schedule::ScheduleKind::Daily,
        UiRuleKind::Weekday => schedule::ScheduleKind::Weekday,
        UiRuleKind::Weekend => schedule::ScheduleKind::Weekend,
        UiRuleKind::Specific => schedule::ScheduleKind::SpecificDate(date.to_string()),
    }
}

fn ui_mode_to_backend(m: UiOptimizeMode) -> schedule::OptimizeMode {
    match m {
        UiOptimizeMode::High => schedule::OptimizeMode::High,
        UiOptimizeMode::Normal => schedule::OptimizeMode::Normal,
        UiOptimizeMode::LowPower => schedule::OptimizeMode::LowPower,
    }
}

pub fn make_rule_ui(r: &schedule::ScheduleRule) -> ScheduleRuleUi {
    ScheduleRuleUi {
        id: r.id as i32,
        summary: r.summary().into(),
        active: r.active,
    }
}

pub fn sync_vec_model(model: &Rc<VecModel<ScheduleRuleUi>>, rules: &[schedule::ScheduleRule]) {
    let cur = model.row_count();
    for (i, r) in rules.iter().enumerate() {
        let ui = make_rule_ui(r);
        if i < cur {
            model.set_row_data(i, ui);
        } else {
            model.push(ui);
        }
    }
    while model.row_count() > rules.len() {
        model.remove(model.row_count() - 1);
    }
}

pub fn sync_rules_ui(
    app: &AppWindow,
    model: &Rc<VecModel<ScheduleRuleUi>>,
    rules: &[schedule::ScheduleRule],
) {
    sync_vec_model(model, rules);
    app.set_schedule_empty(rules.is_empty());
    let info = match schedule::active_rule(rules) {
        Some(r) => format!("활성 규칙: {}", r.summary()),
        None => "활성 규칙 없음.".to_string(),
    };
    app.set_active_rule_info(info.into());
}

pub fn apply_initial(app: &AppWindow, state: &AppState) {
    sync_vec_model(&state.schedule.rules_model, &state.schedule.rules.borrow());
    app.set_schedule_rules(ModelRc::from(state.schedule.rules_model.clone()));
    app.set_schedule_empty(state.schedule.rules.borrow().is_empty());
    let borrow = state.schedule.rules.borrow();
    let info = match schedule::active_rule(&borrow) {
        Some(r) => format!("활성 규칙: {}", r.summary()),
        None => "활성 규칙 없음.".to_string(),
    };
    app.set_active_rule_info(info.into());
}

pub fn register(app: &AppWindow, state: &AppState) {
    app.on_add_schedule_rule({
        let app = app.as_weak();
        let rules = Rc::clone(&state.schedule.rules);
        let rules_model = Rc::clone(&state.schedule.rules_model);
        move || {
            if let Some(app) = app.upgrade() {
                let name = app.get_new_rule_name().to_string();
                let ui_kind = app.get_new_rule_kind();
                let date_str = app.get_new_rule_date().to_string();
                let start = app.get_new_rule_start().to_string();
                let end = app.get_new_rule_end().to_string();
                let ui_mode = app.get_new_rule_mode();

                if name.is_empty() || start.is_empty() || end.is_empty() {
                    push_status(&app, "규칙 이름, 시작/종료 시간을 모두 입력하세요.");
                    return;
                }
                if name.chars().count() > 64 {
                    push_status(&app, "규칙 이름이 너무 깁니다. 64자 이내로 입력하세요.");
                    return;
                }
                if !schedule::validate_time(&start) || !schedule::validate_time(&end) {
                    push_status(
                        &app,
                        "시작/종료 시간 형식이 올바르지 않습니다. HH:MM 형식으로 입력하세요.",
                    );
                    return;
                }
                if ui_kind == UiRuleKind::Specific && !schedule::validate_date(&date_str) {
                    push_status(
                        &app,
                        "날짜 형식이 올바르지 않습니다. YYYY-MM-DD 형식으로 입력하세요.",
                    );
                    return;
                }

                let mut r = rules.borrow_mut();
                let id = schedule::next_id(&r);
                r.push(schedule::ScheduleRule {
                    id,
                    name,
                    kind: ui_kind_to_backend(ui_kind, &date_str),
                    start_time: start,
                    end_time: end,
                    mode: ui_mode_to_backend(ui_mode),
                    active: true,
                });
                schedule::save_rules(&r);
                sync_rules_ui(&app, &rules_model, &r);
                push_status(&app, "스케줄 규칙이 추가되었습니다.");
            }
        }
    });

    app.on_delete_schedule_rule({
        let app = app.as_weak();
        let rules = Rc::clone(&state.schedule.rules);
        let rules_model = Rc::clone(&state.schedule.rules_model);
        move |id| {
            if let Some(app) = app.upgrade() {
                let mut r = rules.borrow_mut();
                r.retain(|rule| rule.id != id as u32);
                schedule::save_rules(&r);
                sync_rules_ui(&app, &rules_model, &r);
                push_status(&app, "스케줄 규칙이 삭제되었습니다.");
            }
        }
    });

    app.on_toggle_schedule_rule({
        let app = app.as_weak();
        let rules = Rc::clone(&state.schedule.rules);
        let rules_model = Rc::clone(&state.schedule.rules_model);
        move |id| {
            if let Some(app) = app.upgrade() {
                let mut r = rules.borrow_mut();
                if let Some(rule) = r.iter_mut().find(|rule| rule.id == id as u32) {
                    rule.active = !rule.active;
                }
                schedule::save_rules(&r);
                sync_rules_ui(&app, &rules_model, &r);
            }
        }
    });
}

pub fn start_auto_mode_timer(app: &AppWindow, state: &AppState) -> slint::Timer {
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(60),
        {
            let app = app.as_weak();
            let rules = Rc::clone(&state.schedule.rules);
            let state = state.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    let r = rules.borrow();
                    let info = match schedule::active_rule(&r) {
                        Some(rule) => {
                            control::apply_mode(&app, &state, rule.mode);
                            format!("활성 규칙: {}", rule.summary())
                        }
                        None => "활성 규칙 없음.".to_string(),
                    };
                    app.set_active_rule_info(info.into());
                }
            }
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    const SOURCE: &str = include_str!("schedule_ui.rs");

    fn start_auto_mode_timer_source() -> &'static str {
        let start = SOURCE
            .find("pub fn start_auto_mode_timer")
            .expect("start_auto_mode_timer must exist");
        let rest = &SOURCE[start..];
        let end = rest.find("#[cfg(test)]").unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn auto_mode_timer_uses_shared_apply_mode_path() {
        let body = start_auto_mode_timer_source();

        assert!(body.contains("control::apply_mode(&app, &state,"));
        assert!(!body.contains("process::apply_optimization"));
    }
}
