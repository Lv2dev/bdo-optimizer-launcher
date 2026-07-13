use super::process::{find_process_id, find_process_id_fresh};
use std::ffi::c_void;
use std::fs::File;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FALSE, HANDLE, HWND};
use windows::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_SIMPLE_DISPLAY_TYPE};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_WHOLECHAIN,
    WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UICONTEXT_EXECUTE, WTD_UI_NONE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_ID_INFO,
    FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{
    CreateProcessW, QueryFullProcessImageNameW, ResumeThread, TerminateProcess,
    WaitForSingleObject, CREATE_SUSPENDED, PROCESS_INFORMATION, PROCESS_NAME_WIN32, STARTUPINFOW,
};

const GAME_EXE: &str = "BlackDesert64.exe";
const LAUNCHER_EXE: &str = "BlackDesertLauncher.exe";
const INSTALL_SUBPATH: &str = "Pearlabyss\\BlackDesert";
const APPROVED_LAUNCHER_PUBLISHERS: [&str; 1] = ["Pearl abyss Corp"];

pub enum LaunchResult {
    GameAlreadyRunning,
    LauncherStarted(PathBuf),
    LauncherRejected(PathBuf, String),
    LauncherNotFound,
}

#[derive(thiserror::Error, Debug)]
enum VerificationError {
    #[error("런처 파일 잠금 실패: {0}")]
    Open(#[from] std::io::Error),
    #[error("Authenticode 서명 검증 실패 (코드 {0:#x})")]
    InvalidSignature(i32),
    #[error("Authenticode 서명자 인증서를 찾을 수 없습니다.")]
    MissingSigner,
    #[error("Authenticode 발행자 이름을 읽을 수 없습니다.")]
    MissingPublisher,
    #[error("승인되지 않은 Authenticode 발행자: {0}")]
    UnapprovedPublisher(String),
    #[error("런처 경로에 허용되지 않는 구성요소가 있습니다: {0}")]
    InvalidPath(PathBuf),
    #[error("런처 경로에 reparse point가 포함되어 있습니다: {0}")]
    ReparsePoint(PathBuf),
    #[error("{operation} 실패: {source}")]
    Windows {
        operation: &'static str,
        source: windows::core::Error,
    },
    #[error("런처 최종 경로 확인 실패: {0}")]
    FinalPath(std::io::Error),
    #[error("실행된 프로세스 이미지가 검증한 런처와 일치하지 않습니다.")]
    ChildImageMismatch,
    #[error("중지 상태 런처 재개 실패")]
    Resume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImageDescriptor {
    final_path: PathBuf,
    identity: FileIdentity,
}

struct VerifiedLauncher {
    _path_guards: Vec<File>,
    _locked_file: File,
    image: ImageDescriptor,
    publisher: String,
}

struct LockedLauncherPath {
    guards: Vec<File>,
    file: File,
    image: ImageDescriptor,
}

struct SuspendedChild {
    process: HANDLE,
    thread: HANDLE,
    resumed: bool,
}

impl SuspendedChild {
    fn resume(mut self) -> Result<(), VerificationError> {
        if unsafe { ResumeThread(self.thread) } == u32::MAX {
            return Err(VerificationError::Resume);
        }
        self.resumed = true;
        Ok(())
    }
}

impl Drop for SuspendedChild {
    fn drop(&mut self) {
        if !self.resumed {
            if let Err(error) = unsafe { TerminateProcess(self.process, 1) } {
                tracing::error!(%error, "failed to terminate rejected suspended launcher");
            } else {
                let _ = unsafe { WaitForSingleObject(self.process, 5_000) };
            }
        }
        unsafe {
            let _ = CloseHandle(self.thread);
            let _ = CloseHandle(self.process);
        }
    }
}

fn publisher_is_approved(publisher: &str) -> bool {
    APPROVED_LAUNCHER_PUBLISHERS
        .iter()
        .any(|approved| publisher.eq_ignore_ascii_case(approved))
}

unsafe fn publisher_from_state(state: HANDLE) -> Result<String, VerificationError> {
    let provider = WTHelperProvDataFromStateData(state);
    if provider.is_null() {
        return Err(VerificationError::MissingSigner);
    }
    let signer = WTHelperGetProvSignerFromChain(provider, 0, FALSE, 0);
    if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
        return Err(VerificationError::MissingSigner);
    }
    let cert = (*(*signer).pasCertChain).pCert;
    if cert.is_null() {
        return Err(VerificationError::MissingSigner);
    }
    let required = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
    if required <= 1 {
        return Err(VerificationError::MissingPublisher);
    }
    let mut buffer = vec![0u16; required as usize];
    let written = CertGetNameStringW(
        cert,
        CERT_NAME_SIMPLE_DISPLAY_TYPE,
        0,
        None,
        Some(&mut buffer),
    );
    if written <= 1 {
        return Err(VerificationError::MissingPublisher);
    }
    Ok(String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

fn windows_error(operation: &'static str, source: windows::core::Error) -> VerificationError {
    VerificationError::Windows { operation, source }
}

fn path_components_to_lock(path: &Path) -> Result<Vec<PathBuf>, VerificationError> {
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        return Err(VerificationError::InvalidPath(path.to_path_buf()));
    }
    let absolute = std::path::absolute(path)?;
    let mut current = PathBuf::new();
    let mut components = Vec::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                current.push(component.as_os_str());
            }
            std::path::Component::Normal(_) => {
                current.push(component.as_os_str());
                components.push(current.clone());
            }
            std::path::Component::CurDir | std::path::Component::ParentDir => {
                return Err(VerificationError::InvalidPath(absolute));
            }
        }
    }
    if components.is_empty() {
        return Err(VerificationError::InvalidPath(absolute));
    }
    Ok(components)
}

fn reject_reparse_attributes(path: &Path, attributes: u32) -> Result<(), VerificationError> {
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(VerificationError::ReparsePoint(path.to_path_buf()));
    }
    Ok(())
}

fn open_locked_component(path: &Path, is_file: bool) -> Result<File, VerificationError> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let desired_access = if is_file {
        FILE_GENERIC_READ.0
    } else {
        FILE_READ_ATTRIBUTES.0
    };
    let share_mode = if is_file {
        FILE_SHARE_READ
    } else {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    };
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if !is_file {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            desired_access,
            share_mode,
            None,
            OPEN_EXISTING,
            flags,
            HANDLE::default(),
        )
    }
    .map_err(|source| windows_error("런처 경로 구성요소 잠금", source))?;
    let file = unsafe { File::from_raw_handle(handle.0) };
    let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileAttributeTagInfo,
            (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    }
    .map_err(|source| windows_error("런처 reparse 속성 확인", source))?;
    reject_reparse_attributes(path, tag.FileAttributes)?;
    Ok(file)
}

fn image_descriptor(file: &File) -> Result<ImageDescriptor, VerificationError> {
    let handle = HANDLE(file.as_raw_handle());
    let mut id = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|source| windows_error("런처 파일 ID 확인", source))?;

    let mut buffer = vec![0u16; 32_768];
    let written = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) };
    if written == 0 || written as usize >= buffer.len() {
        return Err(VerificationError::FinalPath(std::io::Error::last_os_error()));
    }
    let mut final_path = std::ffi::OsString::from_wide(&buffer[..written as usize])
        .to_string_lossy()
        .into_owned();
    if let Some(dos_path) = final_path.strip_prefix(r"\\?\") {
        final_path = dos_path.to_string();
    }
    Ok(ImageDescriptor {
        final_path: PathBuf::from(final_path),
        identity: FileIdentity {
            volume_serial: id.VolumeSerialNumber,
            file_id: id.FileId.Identifier,
        },
    })
}

fn lock_launcher_path(path: &Path) -> Result<LockedLauncherPath, VerificationError> {
    let components = path_components_to_lock(path)?;
    let mut guards = Vec::with_capacity(components.len().saturating_sub(1));
    let mut locked_file = None;
    for (index, component) in components.iter().enumerate() {
        let is_file = index + 1 == components.len();
        let file = open_locked_component(component, is_file)?;
        if is_file {
            locked_file = Some(file);
        } else {
            guards.push(file);
        }
    }
    let file = locked_file.ok_or_else(|| VerificationError::InvalidPath(path.to_path_buf()))?;
    let image = image_descriptor(&file)?;
    Ok(LockedLauncherPath {
        guards,
        file,
        image,
    })
}

fn child_matches_verified(expected: &ImageDescriptor, actual: &ImageDescriptor) -> bool {
    expected.identity == actual.identity
        && expected
            .final_path
            .to_string_lossy()
            .eq_ignore_ascii_case(&actual.final_path.to_string_lossy())
}

fn verify_launcher_file(path: &Path) -> Result<VerifiedLauncher, VerificationError> {
    let locked = lock_launcher_path(path)?;
    let wide_path: Vec<u16> = locked
        .image
        .final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide_path.as_ptr()),
        hFile: HANDLE(locked.file.as_raw_handle()),
        ..WINTRUST_FILE_INFO::default()
    };
    let mut trust_data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        dwUIContext: WTD_UICONTEXT_EXECUTE,
        ..WINTRUST_DATA::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let status = unsafe {
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            (&mut trust_data as *mut WINTRUST_DATA).cast::<c_void>(),
        )
    };
    let publisher_result = if status == 0 {
        unsafe { publisher_from_state(trust_data.hWVTStateData) }
    } else {
        Err(VerificationError::InvalidSignature(status))
    };
    trust_data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        let _ = WinVerifyTrust(
            HWND::default(),
            &mut action,
            (&mut trust_data as *mut WINTRUST_DATA).cast::<c_void>(),
        );
    }
    let publisher = publisher_result?;
    if !publisher_is_approved(&publisher) {
        return Err(VerificationError::UnapprovedPublisher(publisher));
    }
    Ok(VerifiedLauncher {
        _path_guards: locked.guards,
        _locked_file: locked.file,
        image: locked.image,
        publisher,
    })
}

fn create_suspended_launcher(image: &ImageDescriptor) -> Result<SuspendedChild, VerificationError> {
    let application: Vec<u16> = image
        .final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..STARTUPINFOW::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            PWSTR::null(),
            None,
            None,
            FALSE,
            CREATE_SUSPENDED,
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
    }
    .map_err(|source| windows_error("중지 상태 런처 생성", source))?;
    Ok(SuspendedChild {
        process: process.hProcess,
        thread: process.hThread,
        resumed: false,
    })
}

fn child_image_path(process: HANDLE) -> Result<PathBuf, VerificationError> {
    let mut buffer = vec![0u16; 32_768];
    let mut size = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .map_err(|source| windows_error("자식 런처 이미지 경로 확인", source))?;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..size as usize],
    )))
}

fn spawn_verified_launcher(path: &Path) -> Result<(), VerificationError> {
    let verified = verify_launcher_file(path)?;
    tracing::info!(path = %path.display(), publisher = %verified.publisher, "verified game launcher");
    let child = create_suspended_launcher(&verified.image)?;
    let child_path = child_image_path(child.process)?;
    let actual = lock_launcher_path(&child_path)?;
    if !child_matches_verified(&verified.image, &actual.image) {
        return Err(VerificationError::ChildImageMismatch);
    }
    child.resume()
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
    if find_process_id_fresh(GAME_EXE).is_some() {
        return LaunchResult::GameAlreadyRunning;
    }

    if let UserPathDecision::Use(p) = classify_user_path(user_path) {
        if p.exists() {
            return match spawn_verified_launcher(&p) {
                Ok(()) => LaunchResult::LauncherStarted(p),
                Err(error) => LaunchResult::LauncherRejected(p, error.to_string()),
            };
        }
    }

    let mut last_rejection = None;
    for path in find_launcher_fallback() {
        match spawn_verified_launcher(&path) {
            Ok(()) => return LaunchResult::LauncherStarted(path),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "launcher candidate rejected");
                last_rejection = Some((path, error.to_string()));
            }
        }
    }
    last_rejection
        .map(|(path, error)| LaunchResult::LauncherRejected(path, error))
        .unwrap_or(LaunchResult::LauncherNotFound)
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

fn find_launcher_fallback() -> impl Iterator<Item = PathBuf> {
    // 2. 현재 실행 파일 디렉토리
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(LAUNCHER_EXE)))
        .filter(|candidate| candidate.exists() && is_launcher_exe(candidate));

    // 3. C:\~Z:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe
    // M76 (CR-2): 드라이브 풀스캔 결과에도 is_launcher_exe(파일명)를 적용한다.
    // 수용된 잔여 리스크(2026-06-04 리뷰): 고정 드라이브 제한(DRIVE_FIXED)이 없어 removable/
    // network 드라이브의 동명 경로도 후보가 된다. 사용자 입력 경로가 없을 때만 도달하며, 이미
    // 같은 사용자 권한을 전제로 하는 1인 로컬 위협모델상 ROI가 낮아 명시적으로 수용한다.
    // 위협모델이 바뀌면 GetDriveTypeW로 DRIVE_FIXED만 스캔하도록 강화할 것.
    adjacent.into_iter().chain(validated_launcher_candidates(
        drive_scan_candidates(),
        |candidate| candidate.exists(),
    ))
}

fn validated_launcher_candidates(
    candidates: impl IntoIterator<Item = PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    candidates
        .into_iter()
        .filter(|candidate| exists(candidate) && is_launcher_exe(candidate))
        .collect()
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

    #[test]
    fn drive_scan_filters_missing_and_wrongly_named_candidates() {
        let valid = PathBuf::from(r"C:\Games\BlackDesertLauncher.exe");
        let wrong = PathBuf::from(r"D:\Games\malware.exe");
        let missing = PathBuf::from(r"E:\Games\BlackDesertLauncher.exe");
        let candidates =
            validated_launcher_candidates([valid.clone(), wrong, missing], |candidate| {
                candidate != Path::new(r"E:\Games\BlackDesertLauncher.exe")
            });

        assert_eq!(candidates, vec![valid]);
    }

    #[test]
    fn launcher_publisher_allowlist_is_exact_and_case_insensitive() {
        assert!(publisher_is_approved("Pearl abyss Corp"));
        assert!(publisher_is_approved("PEARL ABYSS CORP"));
        assert!(!publisher_is_approved("Pearl abyss Corp Malware"));
        assert!(!publisher_is_approved("Other Signed Vendor"));
    }

    #[test]
    fn child_image_requires_same_final_path_volume_and_file_id() {
        let expected = ImageDescriptor {
            final_path: PathBuf::from(r"C:\Games\BlackDesertLauncher.exe"),
            identity: FileIdentity {
                volume_serial: 7,
                file_id: [3; 16],
            },
        };
        assert!(child_matches_verified(&expected, &expected));

        let different_file = ImageDescriptor {
            final_path: expected.final_path.clone(),
            identity: FileIdentity {
                volume_serial: 7,
                file_id: [4; 16],
            },
        };
        assert!(!child_matches_verified(&expected, &different_file));

        let different_path = ImageDescriptor {
            final_path: PathBuf::from(r"C:\Games\other.exe"),
            identity: expected.identity,
        };
        assert!(!child_matches_verified(&expected, &different_path));
    }

    #[test]
    fn reparse_components_and_parent_segments_are_rejected() {
        assert!(reject_reparse_attributes(
            Path::new(r"C:\Games\link"),
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
        )
        .is_err());
        assert!(
            path_components_to_lock(Path::new(r"C:\Games\..\BlackDesertLauncher.exe")).is_err()
        );
    }

    #[test]
    fn locked_parent_cannot_be_renamed_until_launcher_guard_drops() {
        let root = std::env::temp_dir().join(format!(
            "bdo-launcher-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let parent = root.join("game");
        let renamed = root.join("renamed");
        std::fs::create_dir_all(&parent).unwrap();
        let launcher = parent.join("BlackDesertLauncher.exe");
        std::fs::write(&launcher, b"test").unwrap();

        let locked = lock_launcher_path(&launcher).unwrap();
        assert!(std::fs::rename(&parent, &renamed).is_err());
        drop(locked);
        std::fs::rename(&parent, &renamed).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn suspended_child_image_matches_locked_current_executable() {
        let expected = lock_launcher_path(&std::env::current_exe().unwrap()).unwrap();
        let child = create_suspended_launcher(&expected.image).unwrap();
        let path = child_image_path(child.process).unwrap();
        let actual = lock_launcher_path(&path).unwrap();

        assert!(child_matches_verified(&expected.image, &actual.image));
        drop(child);
    }

    #[test]
    #[ignore = "requires the locally installed official Black Desert launcher"]
    fn installed_launcher_has_valid_approved_authenticode_signature() {
        let path = Path::new(r"C:\Pearlabyss\BlackDesert\BlackDesertLauncher.exe");
        let verified = verify_launcher_file(path).unwrap();

        assert_eq!(verified.publisher, "Pearl abyss Corp");
    }
}
