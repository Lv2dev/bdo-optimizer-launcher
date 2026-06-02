pub mod admin;
pub mod autostart;
pub mod fps;
pub mod launcher;
pub mod logging;
pub mod monitor;
pub mod process;
pub mod schedule;
pub mod settings;
pub mod shutdown;
pub mod singleton;
pub mod system_info;
pub mod tray;
pub mod update;
pub mod window;

#[cfg(windows)]
fn path_from_windows_dir_api(
    api: unsafe fn(Option<&mut [u16]>) -> u32,
) -> Option<std::path::PathBuf> {
    let mut buf = vec![0u16; 32768];
    let len = unsafe { api(Some(&mut buf)) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    Some(std::path::PathBuf::from(String::from_utf16_lossy(
        &buf[..len],
    )))
}

#[cfg(windows)]
fn system_dir() -> std::path::PathBuf {
    path_from_windows_dir_api(windows::Win32::System::SystemInformation::GetSystemDirectoryW)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows\System32"))
}

#[cfg(windows)]
fn windows_dir() -> std::path::PathBuf {
    path_from_windows_dir_api(windows::Win32::System::SystemInformation::GetWindowsDirectoryW)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
}

// 실제 System32\<name>의 절대경로를 반환한다.
// 관리자 권한 프로세스에서 SystemRoot/PATH 오염에 끌려가지 않도록 Windows API를 사용한다.
// 시스템 바이너리(reg.exe, schtasks.exe, shutdown.exe)에 대한 binary planting을 방지한다.
#[cfg(windows)]
pub fn system32_path(name: &str) -> std::path::PathBuf {
    system_dir().join(name)
}

#[cfg(not(windows))]
pub fn system32_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(name)
}

// Windows 디렉터리\<name>의 절대경로를 반환한다. explorer.exe처럼 System32가 아닌
// Windows 루트에 위치한 OS 바이너리를 PATH 탐색 없이 실행할 때 사용한다.
#[cfg(windows)]
pub fn windows_path(name: &str) -> std::path::PathBuf {
    windows_dir().join(name)
}

#[cfg(not(windows))]
pub fn windows_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(name)
}

// 시스템 바이너리(reg.exe/schtasks.exe/logman.exe/shutdown.exe 등) Command 생성 헬퍼.
// system32 절대경로(M25 binary planting 방어) + CREATE_NO_WINDOW(M52 콘솔 깜빡 방어)를
// 일괄 적용한다. 새 외부 명령 호출은 반드시 이 헬퍼를 통해 만든다.
#[cfg(windows)]
pub fn system_command(name: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new(system32_path(name));
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
pub fn system_command(name: &str) -> std::process::Command {
    std::process::Command::new(system32_path(name))
}

// `requireAdministrator` 프로세스에서 spawn하는 자식은 high IL을 상속한다.
// 그래서 사용자가 자유로이 쓸 수 있는 위치의 실행 파일은 신뢰할 수 없다.
// 아래 helper들은 권한 상승 표면이 큰 사용자 쓰기 가능 루트 deny-list를
// autostart, launcher 등 backend 전반에서 공유하기 위한 공통 진입점이다.
// USERPROFILE / APPDATA / LOCALAPPDATA / TEMP / TMP 환경변수 기준.
pub fn high_risk_user_writable_roots() -> Vec<std::path::PathBuf> {
    ["USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .collect()
}

fn normalize_path_for_prefix(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

pub fn path_is_same_or_child(path: &std::path::Path, root: &std::path::Path) -> bool {
    let path = normalize_path_for_prefix(path);
    let root = normalize_path_for_prefix(root);
    if root.is_empty() {
        return false;
    }
    path == root || path.starts_with(&format!("{root}\\"))
}

// path가 deny-list 루트 중 하나와 같거나 그 아래에 있으면 true.
// 임의 커스텀 user-writable 디렉터리는 이 휴리스틱 범위 밖이다(M72b와 동일 한계).
pub fn is_high_risk_user_writable_path(
    path: &std::path::Path,
    roots: &[std::path::PathBuf],
) -> bool {
    roots.iter().any(|root| path_is_same_or_child(path, root))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[cfg(windows)]
    fn system32_path_ignores_poisoned_systemroot_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let original = std::env::var_os("SystemRoot");
        let poisoned = std::path::PathBuf::from(r"C:\Users\attacker");
        std::env::set_var("SystemRoot", &poisoned);

        let path = super::system32_path("schtasks.exe");

        match original {
            Some(v) => std::env::set_var("SystemRoot", v),
            None => std::env::remove_var("SystemRoot"),
        }

        assert_ne!(poisoned.join("System32").join("schtasks.exe"), path);
        assert_eq!(Some(std::ffi::OsStr::new("schtasks.exe")), path.file_name());
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert_eq!("system32", parent);
    }

    #[test]
    #[cfg(windows)]
    fn windows_path_returns_absolute_explorer_path() {
        let path = super::windows_path("explorer.exe");

        assert!(path.is_absolute());
        assert!(path.ends_with("explorer.exe"));
    }

    #[test]
    fn path_is_same_or_child_normalizes_separators_and_case() {
        use std::path::Path;
        let root = Path::new(r"C:\Users\alice");
        assert!(super::path_is_same_or_child(
            Path::new(r"C:\Users\Alice\Downloads\evil.exe"),
            root
        ));
        assert!(super::path_is_same_or_child(
            Path::new("C:/Users/alice/Desktop/x.exe"),
            root
        ));
        assert!(super::path_is_same_or_child(
            Path::new(r"C:\Users\alice\"),
            root
        ));
        assert!(!super::path_is_same_or_child(
            Path::new(r"C:\Program Files\BDO\launcher.exe"),
            root
        ));
        // 동일 prefix를 가지지만 별개 루트("alice2")는 deny되지 않아야 한다.
        assert!(!super::path_is_same_or_child(
            Path::new(r"C:\Users\alice2\file.exe"),
            root
        ));
    }

    #[test]
    fn is_high_risk_user_writable_path_matches_any_root() {
        use std::path::{Path, PathBuf};
        let roots = vec![
            PathBuf::from(r"C:\Users\alice"),
            PathBuf::from(r"C:\Users\alice\AppData\Local\Temp"),
        ];
        assert!(super::is_high_risk_user_writable_path(
            Path::new(r"C:\Users\alice\Downloads\BlackDesertLauncher.exe"),
            &roots
        ));
        assert!(super::is_high_risk_user_writable_path(
            Path::new(r"C:\Users\alice\AppData\Local\Temp\stage\loader.exe"),
            &roots
        ));
        assert!(!super::is_high_risk_user_writable_path(
            Path::new(r"D:\Games\BlackDesert\BlackDesertLauncher.exe"),
            &roots
        ));
        // roots가 비어 있어도 panic 없이 false.
        assert!(!super::is_high_risk_user_writable_path(
            Path::new(r"C:\Users\alice\anything.exe"),
            &[]
        ));
    }
}
