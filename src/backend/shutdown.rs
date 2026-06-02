use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Weekday};

const TASK_ONCE: &str = "BDO_Auto_Shutdown_Once";
const TASK_WEEKLY: &str = "BDO_Auto_Shutdown_Weekly";

// M66a: thiserror 기반 enum. 호출처는 `format!("{e}")` Display로 동일 메시지 유지 → 호출처 변경 0.
// 향후 호출처에서 분기(예: `Error::TaskNotFound` 별도 처리)나 로깅 구조화(`tracing::error!(?e)`) 가능.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    InvalidInput(String),
    #[error("schtasks 실행 실패: {0}")]
    SchtasksSpawn(#[from] std::io::Error),
    #[error("단발 종료 예약 등록 실패. 관리자 권한을 확인하세요.")]
    RegisterOnceFailed,
    #[error("매주 반복 종료 예약 등록 실패. 관리자 권한을 확인하세요.")]
    RegisterWeeklyFailed,
    #[error("이미 등록된 예약이 없습니다.")]
    TaskNotFound,
    #[error("예약 취소 실패. 관리자 권한을 확인하세요. ({0})")]
    DeleteFailed(String),
}

pub struct ScheduleSnapshot {
    pub once: Option<DateTime<Local>>,
    pub weekly: Option<WeeklyInfo>,
}

pub struct WeeklyInfo {
    pub days: Vec<&'static str>,
    pub time_hm: (u32, u32),
    pub next_run: DateTime<Local>,
}

// 작업 스케줄러가 PATH 해석으로 가짜 shutdown.exe를 실행하지 못하도록 절대경로 사용.
fn shutdown_action() -> String {
    format!(
        "{} /s /f /t 0",
        super::system32_path("shutdown.exe").display()
    )
}

fn schtasks_cmd() -> std::process::Command {
    super::system_command("schtasks.exe")
}

pub fn register_once_shutdown(date: &str, time: &str) -> Result<(), Error> {
    validate_date(date)?;
    validate_time(time)?;

    let action = shutdown_action();
    let ok = schtasks_cmd()
        .args([
            "/create", "/tn", TASK_ONCE, "/tr", &action, "/sc", "once", "/sd", date, "/st", time,
            "/f",
        ])
        .output()?
        .status
        .success();

    if ok && task_exists(TASK_ONCE) {
        Ok(())
    } else {
        Err(Error::RegisterOnceFailed)
    }
}

pub fn register_weekly_shutdown(days: &[&str], time: &str) -> Result<(), Error> {
    if days.is_empty() {
        return Err(Error::InvalidInput("요일을 하나 이상 선택하세요.".into()));
    }
    validate_time(time)?;

    let days_str = days.join(",");
    let action = shutdown_action();
    let ok = schtasks_cmd()
        .args([
            "/create",
            "/tn",
            TASK_WEEKLY,
            "/tr",
            &action,
            "/sc",
            "weekly",
            "/d",
            &days_str,
            "/st",
            time,
            "/f",
        ])
        .output()?
        .status
        .success();

    if ok && task_exists(TASK_WEEKLY) {
        Ok(())
    } else {
        Err(Error::RegisterWeeklyFailed)
    }
}

fn task_exists(name: &str) -> bool {
    schtasks_cmd()
        .args(["/query", "/tn", name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// 단일 작업 XML 출력. 작업 미존재 또는 schtasks 자체 실패 시 빈 문자열.
fn fetch_task_xml(name: &str) -> String {
    match schtasks_cmd()
        .args(["/query", "/tn", name, "/xml"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

fn build_local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Option<DateTime<Local>> {
    let nd = NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, 0)?;
    Local.from_local_datetime(&nd).single()
}

/// 두 작업의 등록 상태와 다음 실행 시각을 한 번에 조회.
/// XML substring 파싱 + chrono 직접 계산으로 로케일 무관.
pub fn query_schedules() -> ScheduleSnapshot {
    let once_xml = fetch_task_xml(TASK_ONCE);
    let once =
        parse_start_boundary(&once_xml).and_then(|(y, mo, d, h, mi)| build_local(y, mo, d, h, mi));

    let weekly_xml = fetch_task_xml(TASK_WEEKLY);
    let weekly = match parse_start_boundary(&weekly_xml) {
        Some((_, _, _, h, mi)) => {
            let days = parse_days_of_week(&weekly_xml);
            if days.is_empty() {
                None
            } else {
                let now = Local::now();
                compute_next_weekly_run_from_opt(&days, (h, mi), now).map(|next| WeeklyInfo {
                    days,
                    time_hm: (h, mi),
                    next_run: next,
                })
            }
        }
        None => None,
    };

    ScheduleSnapshot { once, weekly }
}

fn delete_task(task_name: &str) -> Result<(), Error> {
    if !task_exists(task_name) {
        return Err(Error::TaskNotFound);
    }

    let out = schtasks_cmd()
        .args(["/delete", "/tn", task_name, "/f"])
        .output()?;

    if out.status.success() {
        return Ok(());
    }
    let detail = {
        let o = String::from_utf8_lossy(&out.stdout);
        let e = String::from_utf8_lossy(&out.stderr);
        format!("{}{}", o.trim(), e.trim())
    };
    // "작업 없음"은 정상 상황으로 분류해 사용자 혼란을 줄인다.
    let low = detail.to_lowercase();
    if detail.contains("찾을 수 없")
        || detail.contains("존재하지 않")
        || low.contains("cannot find")
        || low.contains("does not exist")
    {
        return Err(Error::TaskNotFound);
    }
    Err(Error::DeleteFailed(detail))
}

pub fn cancel_once() -> Result<(), Error> {
    delete_task(TASK_ONCE)
}

pub fn cancel_weekly() -> Result<(), Error> {
    delete_task(TASK_WEEKLY)
}

/// `<StartBoundary>YYYY-MM-DDTHH:MM:SS</StartBoundary>` 추출.
fn parse_start_boundary(xml: &str) -> Option<(i32, u32, u32, u32, u32)> {
    const KEY: &str = "<StartBoundary>";
    let s = xml.find(KEY)? + KEY.len();
    let rel = xml[s..].find("</StartBoundary>")?;
    let iso = &xml[s..s + rel];
    let dt = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some((dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute()))
}

/// `<DaysOfWeek>` 블록에서 MON..SUN 표준 순서로 추출.
fn parse_days_of_week(xml: &str) -> Vec<&'static str> {
    const KEY_OPEN: &str = "<DaysOfWeek>";
    const KEY_CLOSE: &str = "</DaysOfWeek>";
    let Some(s) = xml.find(KEY_OPEN) else {
        return Vec::new();
    };
    let s = s + KEY_OPEN.len();
    let Some(rel) = xml[s..].find(KEY_CLOSE) else {
        return Vec::new();
    };
    let block = &xml[s..s + rel];
    let table = [
        ("Monday", "MON"),
        ("Tuesday", "TUE"),
        ("Wednesday", "WED"),
        ("Thursday", "THU"),
        ("Friday", "FRI"),
        ("Saturday", "SAT"),
        ("Sunday", "SUN"),
    ];
    table
        .iter()
        .filter(|(name, _)| {
            // Windows schtasks는 "<Wednesday />" (space 포함). 도구 변형 안전하게 양쪽 매칭.
            block.contains(&format!("<{name}/>")) || block.contains(&format!("<{name} />"))
        })
        .map(|(_, code)| *code)
        .collect()
}

fn weekday_from_str(s: &str) -> Option<Weekday> {
    match s {
        "MON" => Some(Weekday::Mon),
        "TUE" => Some(Weekday::Tue),
        "WED" => Some(Weekday::Wed),
        "THU" => Some(Weekday::Thu),
        "FRI" => Some(Weekday::Fri),
        "SAT" => Some(Weekday::Sat),
        "SUN" => Some(Weekday::Sun),
        _ => None,
    }
}

/// `now` 기준 days/time 매주 일정의 다음 실행 시각.
/// days 비어있거나 모두 invalid면 None. DST 모호 시각도 None.
fn compute_next_weekly_run_from_opt(
    days: &[&str],
    (h, m): (u32, u32),
    now: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let targets: Vec<Weekday> = days.iter().filter_map(|d| weekday_from_str(d)).collect();
    if targets.is_empty() {
        return None;
    }
    for offset in 0..=7 {
        let date = (now + chrono::Duration::days(offset)).date_naive();
        if !targets.contains(&date.weekday()) {
            continue;
        }
        let nd = date.and_hms_opt(h, m, 0)?;
        let cand = Local.from_local_datetime(&nd).single()?;
        if cand > now {
            return Some(cand);
        }
    }
    None
}

fn validate_date(date: &str) -> Result<(), Error> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(Error::InvalidInput(
            "날짜 형식이 올바르지 않습니다. 예: 2026-05-20".into(),
        ));
    }
    let y: i32 = date[..4]
        .parse()
        .map_err(|_| Error::InvalidInput("연도 형식 오류.".into()))?;
    let mo: u32 = date[5..7]
        .parse()
        .map_err(|_| Error::InvalidInput("월 형식 오류.".into()))?;
    let da: u32 = date[8..]
        .parse()
        .map_err(|_| Error::InvalidInput("일 형식 오류.".into()))?;
    NaiveDate::from_ymd_opt(y, mo, da)
        .ok_or_else(|| Error::InvalidInput("존재하지 않는 날짜입니다. 예: 2026-05-20".into()))?;
    Ok(())
}

fn validate_time(time: &str) -> Result<(), Error> {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 || parts[0].len() != 2 || parts[1].len() != 2 {
        return Err(Error::InvalidInput(
            "시간 형식이 올바르지 않습니다. 예: 23:30".into(),
        ));
    }
    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| Error::InvalidInput("시 형식 오류.".into()))?;
    let min: u32 = parts[1]
        .parse()
        .map_err(|_| Error::InvalidInput("분 형식 오류.".into()))?;
    if hour > 23 || min > 59 {
        return Err(Error::InvalidInput(
            "시간 범위 오류 (시: 00-23, 분: 00-59).".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap()
    }

    #[test]
    fn next_weekly_today_future_time() {
        // 2026-05-21(목) 10:00 기준, 매주 목/일 22:00 -> 오늘 22:00
        let now = local(2026, 5, 21, 10, 0);
        let next = compute_next_weekly_run_from_opt(&["THU", "SUN"], (22, 0), now).unwrap();
        assert_eq!(next, local(2026, 5, 21, 22, 0));
    }

    #[test]
    fn next_weekly_today_past_time_next_match() {
        // 2026-05-21(목) 23:00 기준, 매주 목/일 22:00 -> 다음 일요일(5/24) 22:00
        let now = local(2026, 5, 21, 23, 0);
        let next = compute_next_weekly_run_from_opt(&["THU", "SUN"], (22, 0), now).unwrap();
        assert_eq!(next, local(2026, 5, 24, 22, 0));
    }

    #[test]
    fn next_weekly_wraps_week() {
        // 2026-05-23(토) 23:00 기준, 매주 화 02:00 -> 5/26(화) 02:00
        let now = local(2026, 5, 23, 23, 0);
        let next = compute_next_weekly_run_from_opt(&["TUE"], (2, 0), now).unwrap();
        assert_eq!(next, local(2026, 5, 26, 2, 0));
    }

    #[test]
    fn next_weekly_empty_days_none() {
        let now = local(2026, 5, 21, 10, 0);
        let next = compute_next_weekly_run_from_opt(&[], (22, 0), now);
        assert_eq!(next, None);
    }

    #[test]
    fn parse_boundary_basic() {
        let xml = r#"<Task><Triggers><TimeTrigger>
            <StartBoundary>2026-05-22T03:00:00</StartBoundary>
            </TimeTrigger></Triggers></Task>"#;
        assert_eq!(parse_start_boundary(xml), Some((2026, 5, 22, 3, 0)));
    }

    #[test]
    fn parse_boundary_missing_none() {
        let xml = "<Task></Task>";
        assert_eq!(parse_start_boundary(xml), None);
    }

    #[test]
    fn parse_days_basic() {
        let xml = r#"<DaysOfWeek><Tuesday/><Thursday/></DaysOfWeek>"#;
        let days = parse_days_of_week(xml);
        assert_eq!(days, vec!["TUE", "THU"]);
    }

    #[test]
    fn parse_days_order_preserved_by_week() {
        // 입력 순서가 어떻든 결과는 MON..SUN 표준 순서.
        let xml = r#"<DaysOfWeek><Sunday/><Monday/><Wednesday/></DaysOfWeek>"#;
        let days = parse_days_of_week(xml);
        assert_eq!(days, vec!["MON", "WED", "SUN"]);
    }

    #[test]
    fn parse_days_no_block_empty() {
        let xml = "<Task></Task>";
        assert_eq!(parse_days_of_week(xml), Vec::<&str>::new());
    }

    #[test]
    fn parse_days_with_space_in_self_close() {
        // 실제 Windows schtasks /xml 출력 형식 (M53 회귀 fix).
        let xml = r#"<DaysOfWeek>
          <Wednesday />
        </DaysOfWeek>"#;
        assert_eq!(parse_days_of_week(xml), vec!["WED"]);
    }

    #[test]
    fn delete_task_uses_query_precheck_before_delete() {
        let src = include_str!("shutdown.rs");
        assert!(src.contains("fn task_exists("));
        assert!(src.contains(r#".args(["/query", "/tn", name])"#));
        assert!(src.contains("if !task_exists(task_name)"));
    }
}
