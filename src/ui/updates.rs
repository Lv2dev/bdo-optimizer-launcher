use crate::backend::update;
use crate::ui::push_status;
use crate::AppWindow;
use slint::ComponentHandle;

pub fn apply_initial(app: &AppWindow) {
    app.set_update_available(false);
    app.set_update_checking(false);
    app.set_update_release_url("".into());
    let status = if update::configured_release_api_url().is_some() {
        "업데이트 확인 전."
    } else {
        "업데이트 채널 미설정."
    };
    app.set_update_status_text(status.into());
}

fn apply_check_result(app: &AppWindow, result: Result<update::UpdateCheck, update::Error>) {
    app.set_update_checking(false);
    match result {
        Ok(check) => {
            app.set_update_status_text(check.status_text.clone().into());
            app.set_update_available(check.update_available);
            app.set_update_release_url(check.release_url.into());
            push_status(app, check.status_text);
        }
        Err(update::Error::ChannelNotConfigured) => {
            app.set_update_available(false);
            app.set_update_release_url("".into());
            app.set_update_status_text("업데이트 채널 미설정.".into());
            push_status(app, "업데이트 채널이 설정되지 않았습니다.");
        }
        Err(e) => {
            let msg = e.to_string();
            app.set_update_available(false);
            app.set_update_release_url("".into());
            app.set_update_status_text(msg.clone().into());
            push_status(app, msg);
        }
    }
}

pub fn register(app: &AppWindow) {
    app.on_check_for_updates({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                if app.get_update_checking() {
                    return;
                }
                app.set_update_checking(true);
                app.set_update_status_text("업데이트 확인 중...".into());
                let app = app.as_weak();
                std::thread::spawn(move || {
                    let result = update::check_latest_release();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app.upgrade() {
                            apply_check_result(&app, result);
                        }
                    });
                });
            }
        }
    });

    app.on_open_update_release({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let url = app.get_update_release_url().to_string();
                if url.trim().is_empty() {
                    push_status(&app, "열 수 있는 릴리스 페이지가 없습니다.");
                    return;
                }
                match update::open_release_page(&url) {
                    Ok(()) => push_status(&app, "GitHub Release 페이지를 열었습니다."),
                    Err(e) => push_status(&app, e.to_string()),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_ui_properties_callbacks_and_card_are_wired() {
        let app_src = include_str!("../../ui/app.slint");
        let settings_src = include_str!("../../ui/tabs/settings.slint");
        let main_src = include_str!("../main.rs");

        assert!(app_src.contains("update-status-text"));
        assert!(app_src.contains("update-release-url"));
        assert!(app_src.contains("callback check-for-updates()"));
        assert!(app_src.contains("callback open-update-release()"));
        assert!(app_src.contains("check-for-updates => { root.check-for-updates(); }"));
        assert!(app_src.contains("open-update-release => { root.open-update-release(); }"));

        assert!(settings_src.contains("callback check-for-updates()"));
        assert!(settings_src.contains("callback open-update-release()"));
        assert!(settings_src.contains("업데이트"));
        assert!(settings_src.contains("업데이트 확인"));
        assert!(settings_src.contains("릴리스 열기"));

        assert!(main_src.contains("ui::updates::apply_initial(&app);"));
        assert!(main_src.contains("ui::updates::register(&app);"));
    }
}
