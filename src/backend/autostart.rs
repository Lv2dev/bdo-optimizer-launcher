// Windows 시작 시 자동 실행. schtasks 로그온 트리거 작업으로 UAC 프롬프트 없이 elevated 실행.
// 작업 이름은 종료 예약(BDO_Auto_Shutdown_*)과 prefix 분리.

use std::path::{Path, PathBuf};

const TASK_NAME: &str = "BDO_Optimizer_Launcher_Autostart";
const MINIMIZED_FLAG: &str = "--minimized";

// M66a: thiserror enum. 호출처는 Display로 동일 메시지 유지.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("실행 파일 경로 확인 실패: {0}")]
    CurrentExe(std::io::Error),
    #[error("실행 파일 경로에 큰따옴표가 포함되어 자동 시작 등록 불가.")]
    QuoteInPath,
    #[error(
        "자동 시작 등록 거부: 실행 파일이 사용자 쓰기 가능성이 높은 위치에 있습니다. Program Files 같은 관리자 전용 위치로 옮긴 뒤 다시 시도하세요. ({0})"
    )]
    UntrustedAutostartPath(PathBuf),
    #[error("schtasks 실행 실패: {0}")]
    SchtasksSpawn(#[from] std::io::Error),
    #[error("자동 시작 등록 실패. 관리자 권한을 확인하세요. ({0})")]
    RegisterFailed(String),
    #[error("이미 등록된 자동 시작이 없습니다.")]
    TaskNotFound,
    #[error("자동 시작 해제 실패. 관리자 권한을 확인하세요. ({0})")]
    UnregisterFailed(String),
}

fn schtasks_cmd() -> std::process::Command {
    super::system_command("schtasks.exe")
}

fn task_exists() -> bool {
    schtasks_cmd()
        .args(["/query", "/tn", TASK_NAME])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

// M76: deny-list 헬퍼는 backend/mod.rs로 승격되어 launcher와 공유한다.
// 본 모듈은 autostart 컨텍스트 메시지(`Error::UntrustedAutostartPath`)로 변환만 담당.
fn validate_autostart_exe_path_for_roots(
    exe: &Path,
    high_risk_roots: &[PathBuf],
) -> Result<(), Error> {
    if super::is_high_risk_user_writable_path(exe, high_risk_roots) {
        return Err(Error::UntrustedAutostartPath(exe.to_path_buf()));
    }
    Ok(())
}

fn build_tr_value_for_exe(
    exe: &Path,
    with_tray: bool,
    high_risk_roots: &[PathBuf],
) -> Result<String, Error> {
    validate_autostart_exe_path_for_roots(exe, high_risk_roots)?;
    let exe_str = exe.to_string_lossy().to_string();
    if exe_str.contains('"') {
        return Err(Error::QuoteInPath);
    }
    if with_tray {
        Ok(format!("\"{}\" {}", exe_str, MINIMIZED_FLAG))
    } else {
        Ok(format!("\"{}\"", exe_str))
    }
}

// 현재 실행 파일의 절대경로를 schtasks /tr 인자 형식으로 만든다.
// 공백 포함 경로 안전성을 위해 큰따옴표로 감싸고, with_tray가 true면 --minimized를 덧붙인다.
// 경로에 `"`가 포함되면 schtasks 내부 재파싱에서 인자 경계가 깨지므로 reject (NTFS에서 `"`는
// 금지 문자지만 symlink/hardlink 경유 비정상 입력 방어).
fn build_tr_value(with_tray: bool) -> Result<String, Error> {
    let exe = std::env::current_exe().map_err(Error::CurrentExe)?;
    let roots = super::high_risk_user_writable_roots();
    build_tr_value_for_exe(&exe, with_tray, &roots)
}

pub fn register_autostart(with_tray: bool) -> Result<(), Error> {
    let tr = build_tr_value(with_tray)?;
    let out = schtasks_cmd()
        .args([
            "/create", "/tn", TASK_NAME, "/tr", &tr, "/sc", "onlogon", "/rl", "HIGHEST", "/f",
        ])
        .output()?;

    if out.status.success() {
        let (registered, minimized) = query_autostart();
        if !registered || minimized != with_tray {
            return Err(Error::RegisterFailed("작업 등록 확인 실패.".into()));
        }
        return Ok(());
    }
    let detail = {
        let o = String::from_utf8_lossy(&out.stdout);
        let e = String::from_utf8_lossy(&out.stderr);
        format!("{}{}", o.trim(), e.trim())
    };
    Err(Error::RegisterFailed(detail))
}

pub fn unregister_autostart() -> Result<(), Error> {
    if !task_exists() {
        return Err(Error::TaskNotFound);
    }

    let out = schtasks_cmd()
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .output()?;

    if out.status.success() {
        return Ok(());
    }
    let detail = {
        let o = String::from_utf8_lossy(&out.stdout);
        let e = String::from_utf8_lossy(&out.stderr);
        format!("{}{}", o.trim(), e.trim())
    };
    // "작업 없음"은 정상 상황으로 분류해 사용자 혼란을 줄인다 (shutdown.rs 패턴).
    let low = detail.to_lowercase();
    if detail.contains("찾을 수 없")
        || detail.contains("존재하지 않")
        || low.contains("cannot find")
        || low.contains("does not exist")
    {
        return Err(Error::TaskNotFound);
    }
    Err(Error::UnregisterFailed(detail))
}

// 단일 schtasks 호출로 등록 여부 + --minimized 인자 포함 여부를 동시에 판정.
// /query /v는 task properties(Task To Run 포함)를 CSV로 출력하므로 substring 검사가 충분.
pub fn query_autostart() -> (bool, bool) {
    let out = match schtasks_cmd()
        .args(["/query", "/fo", "CSV", "/v", "/tn", TASK_NAME])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return (false, false),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let registered = text.contains(TASK_NAME);
    let with_tray = text.contains(MINIMIZED_FLAG);
    (registered, with_tray)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    #[test]
    fn autostart_rejects_user_profile_exe_for_highest_task() {
        let roots = vec![p(r"C:\Users\alice")];
        let err = validate_autostart_exe_path_for_roots(
            Path::new(r"C:\Users\alice\Downloads\bdo-optimizer-launcher.exe"),
            &roots,
        )
        .unwrap_err();

        assert!(matches!(err, Error::UntrustedAutostartPath(_)));
    }

    #[test]
    fn autostart_allows_program_files_exe_for_highest_task() {
        let roots = vec![
            p(r"C:\Users\alice"),
            p(r"C:\Users\alice\AppData\Local\Temp"),
        ];

        validate_autostart_exe_path_for_roots(
            Path::new(r"C:\Program Files\BDO Optimizer\bdo-optimizer-launcher.exe"),
            &roots,
        )
        .unwrap();
    }

    #[test]
    fn build_tr_value_for_exe_rejects_untrusted_location_before_schtasks() {
        let roots = vec![p(r"C:\Users\alice")];
        let err = build_tr_value_for_exe(
            Path::new(r"C:\Users\alice\Desktop\bdo-optimizer-launcher.exe"),
            false,
            &roots,
        )
        .unwrap_err();

        assert!(matches!(err, Error::UntrustedAutostartPath(_)));
    }

    #[test]
    fn register_and_unregister_use_query_prechecks() {
        let src = include_str!("autostart.rs");
        assert!(src.contains("fn task_exists("));
        assert!(src.contains(r#".args(["/query", "/tn", TASK_NAME])"#));
        assert!(src.contains("if !task_exists()"));
        assert!(src.contains("if !registered || minimized != with_tray"));
    }
}
