// 외부 프로세스의 메인 창 hwnd 탐색 및 가시성 판정.
// 검은사막의 "트레이로 보내기"는 메인 창을 ShowWindow(SW_HIDE)로 숨기므로
// IsWindowVisible == FALSE를 저전력 모드 자동 진입 조건으로 사용한다.

use windows::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, TRUE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible, GW_OWNER,
};

struct FindCtx {
    pid: u32,
    result: HWND,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx);
    let mut wpid: u32 = 0;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    if wpid != ctx.pid {
        return TRUE;
    }
    // 메인 창 휴리스틱: GW_OWNER가 null + 타이틀 길이 > 0.
    let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
    if !owner.0.is_null() {
        return TRUE;
    }
    if GetWindowTextLengthW(hwnd) <= 0 {
        return TRUE;
    }
    ctx.result = hwnd;
    FALSE // enumeration 중지
}

/// 지정한 PID의 메인 창 hwnd를 반환한다. 못 찾으면 None.
pub fn find_main_window(pid: u32) -> Option<HWND> {
    unsafe {
        let mut ctx = FindCtx {
            pid,
            result: HWND::default(),
        };
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        if ctx.result.0.is_null() {
            None
        } else {
            Some(ctx.result)
        }
    }
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn foreground_process_id() -> Option<u32> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}
