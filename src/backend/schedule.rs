use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScheduleKind {
    Daily,
    Weekday,
    Weekend,
    SpecificDate(String), // YYYY-MM-DD
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OptimizeMode {
    High,
    Normal,
    LowPower,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRule {
    pub id: u32,
    pub name: String,
    pub kind: ScheduleKind,
    pub start_time: String, // HH:MM
    pub end_time: String,   // HH:MM
    pub mode: OptimizeMode,
    pub active: bool,
}

impl ScheduleRule {
    pub fn kind_label(&self) -> &str {
        match &self.kind {
            ScheduleKind::Daily => "매일",
            ScheduleKind::Weekday => "평일",
            ScheduleKind::Weekend => "주말",
            ScheduleKind::SpecificDate(d) => d.as_str(),
        }
    }

    pub fn mode_label(&self) -> &str {
        match self.mode {
            OptimizeMode::High => "고성능",
            OptimizeMode::Normal => "일반",
            OptimizeMode::LowPower => "저전력",
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{} | {} | {}-{} | {}",
            self.name,
            self.kind_label(),
            self.start_time,
            self.end_time,
            self.mode_label()
        )
    }
}

fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(appdata).join("bdo-optimizer-launcher");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("schedules.json"))
}

pub fn load_rules() -> Vec<ScheduleRule> {
    let path = match config_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<ScheduleRule>>(&content) {
        Ok(rules) => rules
            .into_iter()
            .filter(|r| {
                if !validate_time(&r.start_time) || !validate_time(&r.end_time) {
                    return false;
                }
                if let ScheduleKind::SpecificDate(d) = &r.kind {
                    if !validate_date(d) {
                        return false;
                    }
                }
                true
            })
            .collect(),
        Err(_) => {
            // 깨진 JSON은 빈 배열로 덮어쓰지 않도록 원본을 백업해 보존한다.
            backup_broken(&path);
            Vec::new()
        }
    }
}

fn backup_broken(path: &std::path::Path) {
    use chrono::Local;
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let backup = path.with_extension(format!("json.broken-{stamp}"));
    let _ = std::fs::rename(path, backup);
}

pub fn save_rules(rules: &[ScheduleRule]) {
    if let Some(path) = config_path() {
        if let Ok(json) = serde_json::to_string(rules) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub fn next_id(rules: &[ScheduleRule]) -> u32 {
    rules.iter().map(|r| r.id).max().unwrap_or(0) + 1
}

/// 현재 시각 기준 가장 높은 우선순위의 활성 규칙을 반환.
/// 우선순위: 특정 날짜(3) > 평일/주말(2) > 매일(1), 동일 우선순위에서는 id가 클수록(최신) 우선.
pub fn active_rule(rules: &[ScheduleRule]) -> Option<&ScheduleRule> {
    use chrono::{Datelike, Duration, Local, Timelike};
    let now = Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let time_now = format!("{:02}:{:02}", now.hour(), now.minute());
    let weekday = now.weekday().num_days_from_monday() as u8; // 0=월

    let yesterday = now - Duration::days(1);
    let ydate = yesterday.format("%Y-%m-%d").to_string();
    let yweekday = yesterday.weekday().num_days_from_monday() as u8;

    let mut candidates: Vec<&ScheduleRule> = rules
        .iter()
        .filter(|r| r.active)
        .filter(|r| {
            if !in_time_range(&r.start_time, &r.end_time, &time_now) {
                return false;
            }
            // 야간 범위(start > end)이고 현재 시각이 자정 이후(now < end)이면 전날 기준 판정
            let is_overnight = r.start_time.as_str() > r.end_time.as_str();
            if is_overnight && time_now.as_str() < r.end_time.as_str() {
                matches_kind(r, &ydate, yweekday)
            } else {
                matches_kind(r, &today, weekday)
            }
        })
        .collect();

    // 우선순위 내림차순, 동일 우선순위에서는 id 내림차순(최신 우선)
    candidates.sort_by(|a, b| priority(b).cmp(&priority(a)).then(b.id.cmp(&a.id)));
    candidates.into_iter().next()
}

fn priority(rule: &ScheduleRule) -> u8 {
    match &rule.kind {
        ScheduleKind::SpecificDate(_) => 3,
        ScheduleKind::Weekday | ScheduleKind::Weekend => 2,
        ScheduleKind::Daily => 1,
    }
}

fn matches_kind(rule: &ScheduleRule, today: &str, weekday: u8) -> bool {
    match &rule.kind {
        ScheduleKind::Daily => true,
        ScheduleKind::Weekday => weekday < 5,
        ScheduleKind::Weekend => weekday >= 5,
        ScheduleKind::SpecificDate(d) => d == today,
    }
}

/// 자정 걸침 지원: start > end 이면 자정을 넘는 범위로 해석
fn in_time_range(start: &str, end: &str, now: &str) -> bool {
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// HH:MM 형식 검증 (00:00 ~ 23:59, 두 자리 강제)
pub fn validate_time(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' {
        return false;
    }
    let h: u8 = s[..2].parse().unwrap_or(255);
    let m: u8 = s[3..].parse().unwrap_or(255);
    h < 24 && m < 60
}

/// YYYY-MM-DD 형식 및 달력 유효성 검증 (2026-02-31 같은 날짜 거부)
pub fn validate_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let y: i32 = match s[..4].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mo: u32 = match s[5..7].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let da: u32 = match s[8..].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    chrono::NaiveDate::from_ymd_opt(y, mo, da).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: u32, kind: ScheduleKind, start: &str, end: &str) -> ScheduleRule {
        ScheduleRule {
            id,
            name: format!("r{id}"),
            kind,
            start_time: start.to_string(),
            end_time: end.to_string(),
            mode: OptimizeMode::Normal,
            active: true,
        }
    }

    #[test]
    fn validate_time_accepts_boundary() {
        assert!(validate_time("00:00"));
        assert!(validate_time("23:59"));
        assert!(validate_time("09:30"));
    }

    #[test]
    fn validate_time_rejects_invalid() {
        assert!(!validate_time("24:00"));
        assert!(!validate_time("12:60"));
        assert!(!validate_time("9:30")); // 두 자리 강제
        assert!(!validate_time("09-30"));
        assert!(!validate_time(""));
        assert!(!validate_time("12:345"));
    }

    #[test]
    fn validate_date_calendar_strict() {
        assert!(validate_date("2024-02-29")); // 윤년 OK
        assert!(!validate_date("2025-02-29")); // 평년 reject
        assert!(!validate_date("2026-02-31")); // 달력 초과
        assert!(!validate_date("2026-13-01")); // 월 초과
        assert!(validate_date("2026-12-31"));
    }

    #[test]
    fn validate_date_rejects_invalid_format() {
        assert!(!validate_date("2026/05/22"));
        assert!(!validate_date("26-05-22"));
        assert!(!validate_date(""));
        assert!(!validate_date("2026-5-22"));
    }

    #[test]
    fn matches_kind_daily_always() {
        let r = rule(1, ScheduleKind::Daily, "09:00", "10:00");
        for wd in 0u8..7 {
            assert!(matches_kind(&r, "2026-05-22", wd));
        }
    }

    #[test]
    fn matches_kind_weekday_weekend_split() {
        let wd_rule = rule(1, ScheduleKind::Weekday, "09:00", "10:00");
        let we_rule = rule(2, ScheduleKind::Weekend, "09:00", "10:00");
        for wd in 0u8..5 {
            assert!(matches_kind(&wd_rule, "2026-05-22", wd));
            assert!(!matches_kind(&we_rule, "2026-05-22", wd));
        }
        for wd in 5u8..7 {
            assert!(!matches_kind(&wd_rule, "2026-05-22", wd));
            assert!(matches_kind(&we_rule, "2026-05-22", wd));
        }
    }

    #[test]
    fn matches_kind_specific_date_exact() {
        let r = rule(
            1,
            ScheduleKind::SpecificDate("2026-05-22".into()),
            "09:00",
            "10:00",
        );
        assert!(matches_kind(&r, "2026-05-22", 4));
        assert!(!matches_kind(&r, "2026-05-23", 4));
    }

    #[test]
    fn in_time_range_normal() {
        assert!(in_time_range("09:00", "17:00", "09:00")); // 시작 포함
        assert!(in_time_range("09:00", "17:00", "13:00"));
        assert!(!in_time_range("09:00", "17:00", "17:00")); // 종료 미포함
        assert!(!in_time_range("09:00", "17:00", "08:59"));
        assert!(!in_time_range("09:00", "17:00", "17:01"));
    }

    #[test]
    fn in_time_range_overnight() {
        // 야간 22:00 ~ 06:00
        assert!(in_time_range("22:00", "06:00", "22:00"));
        assert!(in_time_range("22:00", "06:00", "23:59"));
        assert!(in_time_range("22:00", "06:00", "00:00"));
        assert!(in_time_range("22:00", "06:00", "05:59"));
        assert!(!in_time_range("22:00", "06:00", "06:00")); // end 미포함
        assert!(!in_time_range("22:00", "06:00", "21:59"));
        assert!(!in_time_range("22:00", "06:00", "12:00"));
    }

    #[test]
    fn priority_specific_highest_daily_lowest() {
        let daily = rule(1, ScheduleKind::Daily, "09:00", "10:00");
        let weekday = rule(2, ScheduleKind::Weekday, "09:00", "10:00");
        let weekend = rule(3, ScheduleKind::Weekend, "09:00", "10:00");
        let specific = rule(
            4,
            ScheduleKind::SpecificDate("2026-05-22".into()),
            "09:00",
            "10:00",
        );
        assert_eq!(priority(&daily), 1);
        assert_eq!(priority(&weekday), 2);
        assert_eq!(priority(&weekend), 2);
        assert_eq!(priority(&specific), 3);
    }

    #[test]
    fn next_id_increments_max() {
        assert_eq!(next_id(&[]), 1);
        let rules = vec![
            rule(1, ScheduleKind::Daily, "09:00", "10:00"),
            rule(5, ScheduleKind::Daily, "09:00", "10:00"),
            rule(3, ScheduleKind::Daily, "09:00", "10:00"),
        ];
        assert_eq!(next_id(&rules), 6);
    }
}
