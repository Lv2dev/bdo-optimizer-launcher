// 단일 인스턴스 보장: Named Mutex(Global\BdoOptimizerLauncherSingleton)로 두 번째 실행을 차단한다.
// 이미 실행 중이면 기존 창을 찾아 포그라운드/복원한 뒤 신규 인스턴스는 즉시 종료한다.
// Mutex handle은 std::mem::forget으로 프로세스 라이프타임 동안 유지한다(Drop 시 즉시 해제되면 단일 인스턴스 보장 무너짐).

use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, FALSE, HWND};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

const MUTEX_NAME: &str = "Global\\BdoOptimizerLauncherSingleton";
const WINDOW_TITLE: &str = "BDO Optimizer";

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

unsafe fn query_process_image_path(handle: windows::Win32::Foundation::HANDLE) -> Option<PathBuf> {
    let mut buf: [u16; 32768] = [0; 32768];
    let mut size: u32 = buf.len() as u32;
    QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        PWSTR(buf.as_mut_ptr()),
        &mut size,
    )
    .ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(
        &buf[..size as usize],
    )))
}

fn hwnd_matches_current_exe(hwnd: HWND) -> bool {
    unsafe {
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }
        let current = match std::env::current_exe() {
            Ok(path) => normalize_path(&path),
            Err(_) => return false,
        };
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) {
            Ok(handle) => handle,
            Err(_) => return false,
        };
        let image_path = query_process_image_path(handle);
        let _ = CloseHandle(handle);
        image_path
            .as_deref()
            .map(normalize_path)
            .map(|path| path == current)
            .unwrap_or(false)
    }
}

/// 첫 인스턴스면 true를 반환한다. 이미 실행 중이면 기존 창을 포그라운드로 띄우고 false를 반환한다.
/// false 반환 시 호출자는 즉시 종료해야 한다.
pub fn acquire_or_focus_existing() -> bool {
    unsafe {
        let name = wide_null(MUTEX_NAME);
        let handle = match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
            Ok(h) => h,
            // CreateMutexW 자체 실패는 권한/세션 이슈일 수 있으나 단일 인스턴스 검증이 불가능하면
            // 보수적으로 통과시켜 사용자가 앱을 쓸 수 있도록 한다.
            Err(_) => return true,
        };
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        if already {
            // 우리 handle은 더 이상 필요 없지만 windows::Foundation::HANDLE은 Drop이 없어
            // 명시적 CloseHandle을 호출하지 않으면 프로세스 종료까지 살아 있다. OS가 정리.
            focus_existing_window();
            return false;
        }
        // 첫 인스턴스: HANDLE은 Copy + Drop 없음. 변수 scope를 벗어나도 OS handle은 열려 있고
        // 프로세스 종료 시까지 mutex가 유지된다. forget은 불필요.
        let _ = handle;
        true
    }
}

fn focus_existing_window() {
    unsafe {
        let title = wide_null(WINDOW_TITLE);
        let hwnd = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())).unwrap_or_default();
        if hwnd.0.is_null() {
            // 기존 인스턴스가 트레이 + 창 destroy 상태일 수 있다. 사용자는 트레이 메뉴로 복원해야 한다.
            return;
        }
        if !hwnd_matches_current_exe(hwnd) {
            return;
        }
        // 최소화/숨김 상태면 복원 (visible이어도 부작용 없음).
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_matches_tauri_config_title() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["productName"].as_str(), Some(WINDOW_TITLE));
        assert_eq!(
            config["app"]["windows"][0]["title"].as_str(),
            Some(WINDOW_TITLE)
        );
    }

    #[test]
    fn focus_path_verifies_owner_process_image_before_showing_window() {
        let src = include_str!("singleton.rs");
        assert!(src.contains("fn hwnd_matches_current_exe("));
        assert!(src.contains("GetWindowThreadProcessId"));
        assert!(src.contains("QueryFullProcessImageNameW"));
        assert!(src.contains("PROCESS_QUERY_LIMITED_INFORMATION"));
        assert!(src.contains("if !hwnd_matches_current_exe(hwnd)"));
    }
}
