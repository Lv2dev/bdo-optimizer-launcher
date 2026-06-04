use super::process::find_process_id;
use std::path::{Path, PathBuf};

const GAME_EXE: &str = "BlackDesert64.exe";
const LAUNCHER_EXE: &str = "BlackDesertLauncher.exe";
const INSTALL_SUBPATH: &str = "Pearlabyss\\BlackDesert";

pub enum LaunchResult {
    GameAlreadyRunning,
    LauncherStarted(PathBuf),
    LauncherNotFound,
}

// M76: 사용자 입력 경로의 trim/이름 검증을 pure fn으로 격리하기 위해 분리.
// M77: USERPROFILE 하위 정품 설치(Documents/Games, Downloads 등)는 허용하기로 결정.
// M95: 단, 드로퍼가 흔히 쓰는 staging 위치(%APPDATA%/%LOCALAPPDATA%/%TEMP%/%TMP%)만
//      좁게 거부해 elevated 권한 상승 표면을 줄인다(USERPROFILE 전체 deny는 미적용).
enum UserPathDecision {
    Use(PathBuf),
    Ignore,
}

pub fn is_game_running() -> bool {
    find_process_id(GAME_EXE).is_some()
}

pub fn launch_game(user_path: &str) -> LaunchResult {
    // 1순위: 게임 이미 실행 중이면 런처 탐색/실행을 완전히 생략
    if is_game_running() {
        return LaunchResult::GameAlreadyRunning;
    }

    if let UserPathDecision::Use(p) = classify_user_path(user_path) {
        if p.exists() {
            return match std::process::Command::new(&p).spawn() {
                Ok(_) => LaunchResult::LauncherStarted(p),
                Err(_) => LaunchResult::LauncherNotFound,
            };
        }
    }

    match find_launcher_fallback() {
        Some(path) => match std::process::Command::new(&path).spawn() {
            Ok(_) => LaunchResult::LauncherStarted(path),
            Err(_) => LaunchResult::LauncherNotFound,
        },
        None => LaunchResult::LauncherNotFound,
    }
}

// 사용자 입력 경로를 분류한다. p.exists()는 보지 않으며 trim/이름 검증 + staging deny만 한다.
fn classify_user_path(user_path: &str) -> UserPathDecision {
    classify_user_path_with_roots(user_path, &super::high_risk_staging_roots())
}

// 순수 분류 로직: env/fs를 읽지 않고 staging deny-list를 인자로 받아 테스트 가능하게 둔다.
fn classify_user_path_with_roots(user_path: &str, deny_roots: &[PathBuf]) -> UserPathDecision {
    let cleaned = user_path.trim().trim_matches('"');
    if cleaned.is_empty() {
        return UserPathDecision::Ignore;
    }
    let p = PathBuf::from(cleaned);
    if !is_launcher_exe(&p) {
        return UserPathDecision::Ignore;
    }
    // M95: staging 위치(%APPDATA%/%LOCALAPPDATA%/%TEMP%/%TMP%)의 동명 런처를 elevated로
    // spawn하는 권한 상승을 차단한다. USERPROFILE 하위 정품 설치(M77)는 통과한다.
    if super::is_high_risk_user_writable_path(&p, deny_roots) {
        return UserPathDecision::Ignore;
    }
    UserPathDecision::Use(p)
}

fn find_launcher_fallback() -> Option<PathBuf> {
    // 2. 현재 실행 파일 디렉토리
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(LAUNCHER_EXE);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 3. C:\~Z:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe
    // M76 (CR-2): 드라이브 풀스캔 결과에도 is_launcher_exe(파일명)를 적용한다.
    // 수용된 잔여 리스크(2026-06-04 리뷰): 고정 드라이브 제한(DRIVE_FIXED)이 없어 removable/
    // network 드라이브의 동명 경로도 후보가 된다. 사용자 입력 경로가 없을 때만 도달하며, 이미
    // 같은 사용자 권한을 전제로 하는 1인 로컬 위협모델상 ROI가 낮아 명시적으로 수용한다.
    // 위협모델이 바뀌면 GetDriveTypeW로 DRIVE_FIXED만 스캔하도록 강화할 것.
    drive_scan_candidates().find(|candidate| candidate.exists() && is_launcher_exe(candidate))
}

fn drive_scan_candidates() -> impl Iterator<Item = PathBuf> {
    (b'C'..=b'Z').map(|drive| {
        PathBuf::from(format!(
            "{}:\\{}\\{}",
            drive as char, INSTALL_SUBPATH, LAUNCHER_EXE
        ))
    })
}

// 파일명이 BlackDesertLauncher.exe인지 검증 (대소문자 무시).
// 사용자 입력 경로가 임의 .bat/.exe를 가리키더라도 elevated 실행을 방지한다.
fn is_launcher_exe(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case(LAUNCHER_EXE))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn accepts_exact_launcher_name() {
        assert!(is_launcher_exe(&PathBuf::from(
            "C:\\Pearlabyss\\BlackDesert\\BlackDesertLauncher.exe"
        )));
    }

    #[test]
    fn accepts_case_insensitive() {
        assert!(is_launcher_exe(&PathBuf::from(
            "C:\\Games\\blackdesertlauncher.exe"
        )));
        assert!(is_launcher_exe(&PathBuf::from(
            "C:\\Games\\BLACKDESERTLAUNCHER.EXE"
        )));
        assert!(is_launcher_exe(&PathBuf::from("BlackDesertLauncher.exe")));
    }

    #[test]
    fn rejects_other_exe() {
        assert!(!is_launcher_exe(&PathBuf::from(
            "C:\\Pearlabyss\\BlackDesert\\BlackDesert64.exe"
        )));
        assert!(!is_launcher_exe(&PathBuf::from("evil.exe")));
        assert!(!is_launcher_exe(&PathBuf::from("notepad.exe")));
    }

    #[test]
    fn rejects_non_exe_extensions() {
        assert!(!is_launcher_exe(&PathBuf::from("evil.bat")));
        assert!(!is_launcher_exe(&PathBuf::from(
            "BlackDesertLauncher.exe.bat"
        )));
        assert!(!is_launcher_exe(&PathBuf::from("BlackDesertLauncher")));
    }

    #[test]
    fn rejects_empty_and_directory() {
        assert!(!is_launcher_exe(&PathBuf::from("")));
        assert!(!is_launcher_exe(&PathBuf::from("C:\\Games\\")));
    }

    #[test]
    fn rejects_similar_name() {
        // 유사하지만 일치하지 않는 이름
        assert!(!is_launcher_exe(&PathBuf::from("BlackDesertLauncher2.exe")));
        assert!(!is_launcher_exe(&PathBuf::from(
            "MyBlackDesertLauncher.exe"
        )));
    }

    // M76: classify_user_path는 p.exists()를 보지 않는다. 분류 로직은
    // classify_user_path_with_roots로 분리해 staging deny-list를 인자 주입으로 테스트한다.
    #[test]
    fn classify_empty_or_whitespace_is_ignored() {
        assert!(matches!(classify_user_path(""), UserPathDecision::Ignore));
        assert!(matches!(
            classify_user_path("   "),
            UserPathDecision::Ignore
        ));
        assert!(matches!(
            classify_user_path("\" \""),
            UserPathDecision::Ignore
        ));
    }

    #[test]
    fn classify_wrong_filename_is_ignored() {
        assert!(matches!(
            classify_user_path(r"C:\Program Files\BDO\notepad.exe"),
            UserPathDecision::Ignore
        ));
        assert!(matches!(
            classify_user_path(r"C:\Program Files\BDO\BlackDesertLauncher.bat"),
            UserPathDecision::Ignore
        ));
    }

    // M77: 정상 사용자 시나리오 — USERPROFILE 아래(예: Documents/Games/...)에
    // 설치한 정품 BlackDesertLauncher.exe도 통과해야 한다.
    #[test]
    fn classify_user_profile_launcher_is_used() {
        let decision =
            classify_user_path(r"C:\Users\alice\Documents\Games\BDO\BlackDesertLauncher.exe");
        assert!(matches!(decision, UserPathDecision::Use(_)));
    }

    #[test]
    fn classify_other_drive_launcher_is_used() {
        let decision = classify_user_path(r"D:\Games\BlackDesert\BlackDesertLauncher.exe");
        assert!(matches!(decision, UserPathDecision::Use(_)));
    }

    #[test]
    fn classify_strips_quotes_and_whitespace() {
        let decision = classify_user_path("  \"D:\\Games\\BDO\\BlackDesertLauncher.exe\"  ");
        match decision {
            UserPathDecision::Use(p) => {
                assert_eq!(p, PathBuf::from(r"D:\Games\BDO\BlackDesertLauncher.exe"));
            }
            _ => panic!("trim/quote stripping 실패"),
        }
    }

    // M95: staging 위치(%APPDATA%/%TEMP% 등)의 동명 런처는 elevated 실행에서 거부(Ignore).
    #[test]
    fn classify_rejects_staging_dir_launcher() {
        let roots = vec![
            PathBuf::from(r"C:\Users\alice\AppData\Local\Temp"),
            PathBuf::from(r"C:\Users\alice\AppData\Roaming"),
        ];
        assert!(matches!(
            classify_user_path_with_roots(
                r"C:\Users\alice\AppData\Local\Temp\stage\BlackDesertLauncher.exe",
                &roots
            ),
            UserPathDecision::Ignore
        ));
    }

    // M95/M77: staging 밖 프로필 설치(Documents/Games, Downloads)는 계속 허용(Use).
    #[test]
    fn classify_allows_profile_install_outside_staging() {
        let roots = vec![
            PathBuf::from(r"C:\Users\alice\AppData\Local\Temp"),
            PathBuf::from(r"C:\Users\alice\AppData\Roaming"),
        ];
        assert!(matches!(
            classify_user_path_with_roots(
                r"C:\Users\alice\Documents\Games\BDO\BlackDesertLauncher.exe",
                &roots
            ),
            UserPathDecision::Use(_)
        ));
        assert!(matches!(
            classify_user_path_with_roots(
                r"C:\Users\alice\Downloads\BlackDesertLauncher.exe",
                &roots
            ),
            UserPathDecision::Use(_)
        ));
    }

    // M76 (CR-2): 드라이브 풀스캔이 후보 경로의 이름 검증을 누락하지 않는지
    // 소스 텍스트로 회귀 잠금. M71 패턴 — 런타임 환경 의존 없이 invariant 보호.
    #[test]
    fn drive_scan_calls_is_launcher_exe_on_each_candidate() {
        let source = include_str!("launcher.rs");
        let scan_block = source
            .split("fn find_launcher_fallback")
            .nth(1)
            .expect("find_launcher_fallback 함수가 누락됨");
        assert!(
            scan_block.contains("drive_scan_candidates"),
            "fallback이 drive_scan_candidates iterator를 쓰지 않음"
        );
        assert!(
            scan_block.contains("is_launcher_exe(candidate)"),
            "drive scan 결과에 is_launcher_exe 검증이 빠짐 (CR-2 회귀)"
        );
    }
}
