// Windows 시작 시 자동 실행. Task Scheduler 로그온 트리거로 UAC 프롬프트 없이 elevated 실행.
// 작업 이름은 종료 예약(BDO_Auto_Shutdown_*)과 prefix 분리.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::{BSTR, PCWSTR, VARIANT};
use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL, RPC_E_CHANGED_MODE};
use windows::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    AclSizeInformation, CreateWellKnownSid, EqualSid, GetAce, GetAclInformation, MapGenericMask,
    WinBuiltinAdministratorsSid, WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, GENERIC_MAPPING, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_MAX_SID_SIZE, WELL_KNOWN_SID_TYPE,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    WRITE_DAC, WRITE_OWNER,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::TaskScheduler::{
    ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN,
};
use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

const TASK_NAME: &str = "BDO_Optimizer_Launcher_Autostart";
const MINIMIZED_FLAG: &str = "--minimized";
const LOGON_DELAY: &str = "PT30S";
const EXECUTION_TIME_LIMIT: &str = "PT0S";
const RESTART_INTERVAL: &str = "PT1M";
const RESTART_COUNT: &str = "3";
static AUTOSTART_OPERATION_LOCK: Mutex<()> = Mutex::new(());

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
    #[error("Program Files 경로 확인 실패: {0}")]
    ProgramFiles(windows::core::Error),
    #[error("Program Files 경로 문자열 변환 실패: {0}")]
    ProgramFilesEncoding(String),
    #[error("자동 시작 경로 정규화 실패 ({path}): {source}")]
    Canonicalize {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("자동 시작 보안 정보 조회 실패 ({path}, 코드 {code})")]
    SecurityInfo { path: PathBuf, code: u32 },
    #[error("자동 시작 SID 생성 실패: {0}")]
    Sid(windows::core::Error),
    #[error("자동 시작 ACL 구조 확인 실패: {0}")]
    Acl(windows::core::Error),
    #[error("자동 시작 등록 거부: 일반 사용자가 수정 가능한 경로입니다. ({0})")]
    WritableDacl(PathBuf),
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

fn task_file_path() -> PathBuf {
    super::windows_path("System32")
        .join("Tasks")
        .join(TASK_NAME)
}

fn task_presence_from_query(
    query_succeeded: bool,
    task_file_exists: Result<bool, String>,
    detail: String,
) -> Result<bool, Error> {
    if query_succeeded {
        return Ok(true);
    }
    match task_file_exists {
        Ok(false) => Ok(false),
        Ok(true) => Err(Error::UnregisterFailed(format!("작업 조회 실패: {detail}"))),
        Err(file_error) => Err(Error::UnregisterFailed(format!(
            "작업 조회 실패: {detail}; 작업 파일 확인 실패: {file_error}"
        ))),
    }
}

fn task_exists() -> Result<bool, Error> {
    let out = schtasks_cmd().args(["/query", "/tn", TASK_NAME]).output()?;
    let detail = {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = format!("{}{}", stdout.trim(), stderr.trim());
        if detail.is_empty() {
            "schtasks /query가 비정상 종료했습니다.".to_string()
        } else {
            detail
        }
    };
    let task_file_exists = task_file_path()
        .try_exists()
        .map_err(|error| error.to_string());
    task_presence_from_query(out.status.success(), task_file_exists, detail)
}

fn task_deletion_is_confirmed(query_succeeded: bool, task_file_exists: bool) -> bool {
    !query_succeeded && !task_file_exists
}

fn require_existing_task(exists: bool) -> Result<(), Error> {
    exists.then_some(()).ok_or(Error::TaskNotFound)
}

fn registration_matches(registered: bool, minimized: bool, expected_minimized: bool) -> bool {
    registered && minimized == expected_minimized
}

fn verify_registration_with(
    expected_minimized: bool,
    query: impl FnOnce() -> (bool, bool),
    cleanup: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    let (registered, minimized) = query();
    if registration_matches(registered, minimized, expected_minimized) {
        return Ok(());
    }
    match cleanup() {
        Ok(()) | Err(Error::TaskNotFound) => {
            Err(Error::RegisterFailed("작업 등록 확인 실패.".into()))
        }
        Err(error) => Err(Error::RegisterFailed(format!(
            "작업 등록 확인 실패 후 잘못된 작업 정리에도 실패했습니다: {error}"
        ))),
    }
}

fn unregister_with(
    exists: impl FnOnce() -> Result<bool, Error>,
    delete: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    require_existing_task(exists()?)?;
    delete()
}

// M76: deny-list 헬퍼는 backend/mod.rs로 승격되어 launcher와 공유한다.
// 본 모듈은 autostart 컨텍스트 메시지(`Error::UntrustedAutostartPath`)로 변환만 담당.
#[cfg(test)]
fn validate_autostart_exe_path_for_roots(
    exe: &Path,
    high_risk_roots: &[PathBuf],
) -> Result<(), Error> {
    if super::is_high_risk_user_writable_path(exe, high_risk_roots) {
        return Err(Error::UntrustedAutostartPath(exe.to_path_buf()));
    }
    Ok(())
}

fn map_file_generic_rights(mut rights: u32) -> u32 {
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: FILE_ALL_ACCESS.0,
    };
    unsafe { MapGenericMask(&mut rights, &mapping) };
    rights
}

fn has_dangerous_write_rights(rights: u32) -> bool {
    let rights = map_file_generic_rights(rights);
    let dangerous = FILE_WRITE_DATA.0
        | FILE_APPEND_DATA.0
        | FILE_WRITE_EA.0
        | FILE_WRITE_ATTRIBUTES.0
        | FILE_DELETE_CHILD.0
        | DELETE.0
        | WRITE_DAC.0
        | WRITE_OWNER.0;
    rights & dangerous != 0
}

const SID_WORDS: usize = (SECURITY_MAX_SID_SIZE as usize).div_ceil(std::mem::size_of::<u64>());
const TRUSTED_INSTALLER_SID: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

struct AlignedSid([u64; SID_WORDS]);

impl AlignedSid {
    fn well_known(sid_type: WELL_KNOWN_SID_TYPE) -> Result<Self, Error> {
        let mut value = Self([0; SID_WORDS]);
        let mut size = SECURITY_MAX_SID_SIZE;
        unsafe { CreateWellKnownSid(sid_type, None, value.as_psid(), &mut size) }
            .map_err(Error::Sid)?;
        Ok(value)
    }

    fn as_psid(&mut self) -> PSID {
        PSID(self.0.as_mut_ptr().cast())
    }

    fn psid(&self) -> PSID {
        PSID(self.0.as_ptr().cast_mut().cast())
    }
}

struct LocalSid(PSID);

impl LocalSid {
    fn from_string(value: &str) -> Result<Self, Error> {
        let wide: Vec<u16> = std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut sid = PSID::default();
        unsafe { ConvertStringSidToSidW(PCWSTR(wide.as_ptr()), &mut sid) }.map_err(Error::Sid)?;
        Ok(Self(sid))
    }
}

impl Drop for LocalSid {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(HLOCAL(self.0 .0));
        }
    }
}

struct TrustedSids {
    system: AlignedSid,
    administrators: AlignedSid,
    trusted_installer: LocalSid,
}

impl TrustedSids {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            system: AlignedSid::well_known(WinLocalSystemSid)?,
            administrators: AlignedSid::well_known(WinBuiltinAdministratorsSid)?,
            trusted_installer: LocalSid::from_string(TRUSTED_INSTALLER_SID)?,
        })
    }

    fn contains(&self, sid: PSID) -> bool {
        [
            self.system.psid(),
            self.administrators.psid(),
            self.trusted_installer.0,
        ]
        .into_iter()
        .any(|trusted| unsafe { EqualSid(sid, trusted) }.is_ok())
    }
}

fn allow_ace(ace: *mut std::ffi::c_void) -> Result<Option<(PSID, u32)>, Error> {
    let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
    if header.AceFlags & 0x08 != 0 {
        return Ok(Some((PSID::default(), 0)));
    }
    let ace_size = header.AceSize as usize;
    let sid_offset = match header.AceType {
        0 | 9 => std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart),
        5 | 11 => {
            if ace_size < 12 {
                return Ok(None);
            }
            let flags = unsafe { *(ace.cast::<u8>().add(8).cast::<u32>()) };
            12 + usize::from(flags & 1 != 0) * 16 + usize::from(flags & 2 != 0) * 16
        }
        4 => return Ok(None),
        _ => return Ok(Some((PSID::default(), 0))),
    };
    if sid_offset >= ace_size {
        return Ok(None);
    }
    let sid = PSID(unsafe { ace.cast::<u8>().add(sid_offset).cast() });
    let mask = unsafe { *(ace.cast::<u8>().add(4).cast::<u32>()) };
    Ok(Some((sid, mask)))
}

fn dacl_has_untrusted_write_access(dacl: *const ACL, trusted: &TrustedSids) -> Result<bool, Error> {
    let mut info = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .map_err(Error::Acl)?;

    for index in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        unsafe { GetAce(dacl, index, &mut ace) }.map_err(Error::Acl)?;
        let Some((sid, rights)) = allow_ace(ace)? else {
            return Ok(true);
        };
        if sid.is_invalid() {
            continue;
        }
        if has_dangerous_write_rights(rights) && !trusted.contains(sid) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn security_descriptor_has_untrusted_write_access(
    owner: PSID,
    dacl: *const ACL,
) -> Result<bool, Error> {
    if owner.is_invalid() || dacl.is_null() {
        return Ok(true);
    }
    let trusted = TrustedSids::new()?;
    if !trusted.contains(owner) {
        return Ok(true);
    }
    dacl_has_untrusted_write_access(dacl, &trusted)
}

fn path_has_untrusted_write_access(path: &Path) -> Result<bool, Error> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(Error::SecurityInfo {
            path: path.to_path_buf(),
            code: status.0,
        });
    }
    let result = security_descriptor_has_untrusted_write_access(owner, dacl);
    unsafe {
        let _ = LocalFree(HLOCAL(descriptor.0));
    }
    result
}

fn program_files_root() -> Result<PathBuf, Error> {
    let ptr = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_ProgramFiles,
            KNOWN_FOLDER_FLAG(0),
            HANDLE::default(),
        )
    }
    .map_err(Error::ProgramFiles)?;
    let text = unsafe { ptr.to_string() };
    unsafe { CoTaskMemFree(Some(ptr.0.cast())) };
    let text = text.map_err(|error| Error::ProgramFilesEncoding(error.to_string()))?;
    Ok(PathBuf::from(text))
}

fn validate_protected_autostart_path(exe: &Path) -> Result<(), Error> {
    let root = program_files_root()?;
    let root = std::fs::canonicalize(&root).map_err(|source| Error::Canonicalize {
        path: root.clone(),
        source,
    })?;
    let exe = std::fs::canonicalize(exe).map_err(|source| Error::Canonicalize {
        path: exe.to_path_buf(),
        source,
    })?;
    if !super::path_is_same_or_child(&exe, &root) {
        return Err(Error::UntrustedAutostartPath(exe));
    }

    let mut current = exe.as_path();
    loop {
        if path_has_untrusted_write_access(current)? {
            return Err(Error::WritableDacl(current.to_path_buf()));
        }
        if current == root {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| Error::UntrustedAutostartPath(exe.clone()))?;
    }
    Ok(())
}

#[cfg(test)]
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

fn validated_current_exe() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe().map_err(Error::CurrentExe)?;
    validate_protected_autostart_path(&exe)?;
    if exe.to_string_lossy().contains('"') {
        return Err(Error::QuoteInPath);
    }
    Ok(exe)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn build_task_xml(exe: &Path, with_tray: bool) -> String {
    let arguments = if with_tray {
        format!("<Arguments>{MINIMIZED_FLAG}</Arguments>")
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>BDO Optimizer 자동 시작</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><Delay>{LOGON_DELAY}</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <Enabled>true</Enabled>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>{EXECUTION_TIME_LIMIT}</ExecutionTimeLimit>
    <RestartOnFailure><Interval>{RESTART_INTERVAL}</Interval><Count>{RESTART_COUNT}</Count></RestartOnFailure>
  </Settings>
  <Actions Context="Author"><Exec><Command>{}</Command>{arguments}</Exec></Actions>
</Task>"#,
        xml_escape(&exe.to_string_lossy())
    )
}

struct ComInitialization(bool);

impl ComInitialization {
    fn initialize() -> Result<Self, Error> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            // Tauri가 다른 apartment 모델로 이미 초기화한 스레드에서도 기존 COM을 그대로 사용한다.
            Ok(Self(false))
        } else {
            Err(Error::RegisterFailed(format!(
                "COM 초기화 실패: {result:?}"
            )))
        }
    }
}

impl Drop for ComInitialization {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn register_task_xml_named(task_name: &str, xml: &str) -> Result<(), Error> {
    let _com = ComInitialization::initialize()?;
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
            let empty = VARIANT::default();
            service.Connect(&empty, &empty, &empty, &empty)?;
            let folder = service.GetFolder(&BSTR::from("\\"))?;
            let user = VARIANT::from(service.ConnectedUser()?);
            folder.RegisterTask(
                &BSTR::from(task_name),
                &BSTR::from(xml),
                TASK_CREATE_OR_UPDATE.0,
                &user,
                &empty,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &empty,
            )?;
            Ok(())
        })()
    };
    result.map_err(|error| Error::RegisterFailed(error.to_string()))
}

fn with_operation_lock<T>(lock: &Mutex<()>, operation: impl FnOnce() -> T) -> T {
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    operation()
}

fn with_autostart_operation<T>(operation: impl FnOnce() -> T) -> T {
    with_operation_lock(&AUTOSTART_OPERATION_LOCK, operation)
}

fn register_autostart_unlocked(with_tray: bool) -> Result<(), Error> {
    let exe = validated_current_exe()?;
    register_task_xml_named(TASK_NAME, &build_task_xml(&exe, with_tray))?;

    verify_registration_with(with_tray, query_autostart, delete_registered_task)
}

pub fn register_autostart(with_tray: bool) -> Result<(), Error> {
    with_autostart_operation(|| register_autostart_unlocked(with_tray))
}

pub fn unregister_autostart() -> Result<(), Error> {
    with_autostart_operation(|| unregister_with(task_exists, delete_registered_task))
}

fn delete_registered_task() -> Result<(), Error> {
    let out = schtasks_cmd()
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .output()?;

    if out.status.success() {
        let query = schtasks_cmd().args(["/query", "/tn", TASK_NAME]).output()?;
        if task_deletion_is_confirmed(query.status.success(), task_file_path().exists()) {
            return Ok(());
        }
        return Err(Error::UnregisterFailed(
            "삭제 명령 성공 후에도 작업이 남아 있습니다.".to_string(),
        ));
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

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    Some(
        xml[start..end]
            .trim()
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">"),
    )
}

fn xml_element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let needle = format!("<{tag}");
    let mut offset = 0;
    while let Some(relative) = xml[offset..].find(&needle) {
        let start = offset + relative;
        let boundary = xml.as_bytes().get(start + needle.len()).copied()?;
        if boundary != b'>' && boundary != b'/' && !boundary.is_ascii_whitespace() {
            offset = start + needle.len();
            continue;
        }
        let open_end = start + xml[start..].find('>')?;
        if xml[start..open_end].trim_end().ends_with('/') {
            return Some("");
        }
        let content_start = open_end + 1;
        let close = format!("</{tag}>");
        let content_end = content_start + xml[content_start..].find(&close)?;
        return Some(&xml[content_start..content_end]);
    }
    None
}

fn xml_enabled_or_default(element: &str) -> bool {
    xml_tag(element, "Enabled")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

fn xml_direct_child_names(xml: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut offset = 0usize;
    while let Some(relative) = xml[offset..].find('<') {
        let start = offset + relative;
        if xml[start..].starts_with("<!--") {
            offset = start + xml[start + 4..].find("-->")? + 7;
            continue;
        }
        let end = start + xml[start..].find('>')?;
        let raw = xml[start + 1..end].trim();
        if raw.starts_with('?') || raw.starts_with('!') {
            offset = end + 1;
            continue;
        }
        if raw.starts_with('/') {
            depth = depth.checked_sub(1)?;
        } else {
            let self_closing = raw.ends_with('/');
            let name = raw.trim_end_matches('/').split_ascii_whitespace().next()?;
            if depth == 0 {
                names.push(name.rsplit(':').next()?.to_string());
            }
            if !self_closing {
                depth += 1;
            }
        }
        offset = end + 1;
    }
    (depth == 0).then_some(names)
}

fn normalize_task_command(command: &str) -> Option<String> {
    let command = command.trim();
    let starts_quoted = command.starts_with('"');
    let ends_quoted = command.ends_with('"');
    let normalized = match (starts_quoted, ends_quoted) {
        (true, true) if command.len() >= 2 => &command[1..command.len() - 1],
        (false, false) => command,
        _ => return None,
    };
    (!normalized.is_empty() && !normalized.contains('"')).then(|| normalized.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeContract {
    Current,
    LegacyResilience,
}

fn runtime_contract(logon_trigger: &str, settings: &str) -> Option<RuntimeContract> {
    let delay = xml_tag(logon_trigger, "Delay");
    let execution_limit = xml_tag(settings, "ExecutionTimeLimit");
    let restart = xml_element(settings, "RestartOnFailure");
    let instance_policy = xml_tag(settings, "MultipleInstancesPolicy");
    let disallow_on_battery = xml_tag(settings, "DisallowStartIfOnBatteries");
    let stop_on_battery = xml_tag(settings, "StopIfGoingOnBatteries");
    let timing_is_current = delay.as_deref() == Some(LOGON_DELAY)
        && execution_limit.as_deref() == Some(EXECUTION_TIME_LIMIT)
        && restart.is_some_and(|value| {
            xml_tag(value, "Interval").as_deref() == Some(RESTART_INTERVAL)
                && xml_tag(value, "Count").as_deref() == Some(RESTART_COUNT)
        });
    let battery_is_current = disallow_on_battery.as_deref() == Some("false")
        && stop_on_battery.as_deref() == Some("false");
    let instance_policy_is_current = instance_policy.as_deref() == Some("IgnoreNew");
    if timing_is_current && battery_is_current && instance_policy_is_current {
        return Some(RuntimeContract::Current);
    }

    let has_known_battery_defaults =
        [&disallow_on_battery, &stop_on_battery]
            .into_iter()
            .all(|value| {
                value
                    .as_deref()
                    .is_none_or(|value| matches!(value, "true" | "false"))
            });
    let has_known_instance_policy = instance_policy
        .as_deref()
        .is_none_or(|value| matches!(value, "Parallel" | "Queue" | "IgnoreNew" | "StopExisting"));
    let has_legacy_time_limit = delay.is_none()
        && restart.is_none()
        && execution_limit
            .as_deref()
            .is_none_or(|value| value == "PT72H");
    (has_legacy_time_limit
        || (timing_is_current && has_known_battery_defaults && has_known_instance_policy))
        .then_some(RuntimeContract::LegacyResilience)
}

fn parse_task_action_contract(xml: &str) -> Option<((String, String), RuntimeContract)> {
    let principals = xml_element(xml, "Principals")?;
    if xml_tag(principals, "LogonType")?.as_str() != "InteractiveToken" {
        return None;
    }
    if xml_tag(principals, "RunLevel")?.as_str() != "HighestAvailable" {
        return None;
    }
    let triggers = xml_element(xml, "Triggers")?;
    if xml_direct_child_names(triggers)? != ["LogonTrigger"] {
        return None;
    }
    let logon_trigger = xml_element(triggers, "LogonTrigger")?;
    if !xml_enabled_or_default(logon_trigger) {
        return None;
    }
    let settings = xml_element(xml, "Settings")?;
    if !xml_enabled_or_default(settings) {
        return None;
    }
    let runtime = runtime_contract(logon_trigger, settings)?;
    let actions = xml_element(xml, "Actions")?;
    if xml_direct_child_names(actions)? != ["Exec"] {
        return None;
    }
    let exec = xml_element(actions, "Exec")?;
    let command = normalize_task_command(&xml_tag(exec, "Command")?)?;
    let arguments = xml_tag(exec, "Arguments").unwrap_or_default();
    Some(((command, arguments), runtime))
}

fn parse_task_action(xml: &str) -> Option<(String, String)> {
    let (action, runtime) = parse_task_action_contract(xml)?;
    (runtime == RuntimeContract::Current).then_some(action)
}

fn parse_legacy_task_action(xml: &str) -> Option<(String, String)> {
    let (action, runtime) = parse_task_action_contract(xml)?;
    (runtime == RuntimeContract::LegacyResilience).then_some(action)
}

fn action_matches_current_exe(command: &str, arguments: &str) -> Option<bool> {
    let expected = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok())?;
    let command = std::fs::canonicalize(PathBuf::from(command)).ok()?;
    if command != expected || validate_protected_autostart_path(&command).is_err() {
        return None;
    }
    let arguments = arguments.trim();
    if !arguments.is_empty() && arguments != MINIMIZED_FLAG {
        return None;
    }
    Some(arguments == MINIMIZED_FLAG)
}

fn query_task_xml() -> Option<String> {
    let out = schtasks_cmd()
        .args(["/query", "/tn", TASK_NAME, "/xml"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// 작업 존재뿐 아니라 action이 현재 보호 설치본과 정확히 일치하는지 확인한다.
pub fn query_autostart() -> (bool, bool) {
    let Some(text) = query_task_xml() else {
        return (false, false);
    };
    let Some((command, arguments)) = parse_task_action(&text) else {
        return (false, false);
    };
    action_matches_current_exe(&command, &arguments)
        .map(|minimized| (true, minimized))
        .unwrap_or((false, false))
}

// 구버전 schtasks 작업만 안전하게 판별하여 새 계약으로 교체한다.
// 다른 실행 파일/인자/트리거를 가진 작업은 고정 이름이 같아도 덮어쓰지 않는다.
pub fn repair_legacy_autostart() -> Result<bool, Error> {
    with_autostart_operation(|| {
        let Some(text) = query_task_xml() else {
            return Ok(false);
        };
        let Some((command, arguments)) = parse_legacy_task_action(&text) else {
            return Ok(false);
        };
        let Some(with_tray) = action_matches_current_exe(&command, &arguments) else {
            return Ok(false);
        };
        register_autostart_unlocked(with_tray)?;
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{GetSecurityDescriptorDacl, GetSecurityDescriptorOwner};

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
    fn autostart_dacl_rejects_effective_write_delete_or_owner_rights() {
        assert!(has_dangerous_write_rights(
            windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE.0
        ));
        assert!(has_dangerous_write_rights(DELETE.0));
        assert!(has_dangerous_write_rights(WRITE_DAC.0));
        assert!(has_dangerous_write_rights(WRITE_OWNER.0));
        assert!(has_dangerous_write_rights(
            windows::Win32::Foundation::GENERIC_ALL.0
        ));
        assert!(!has_dangerous_write_rights(0x0012_00a9));
    }

    fn sddl_has_untrusted_write_access(sddl: &str) -> bool {
        let wide: Vec<u16> = std::ffi::OsStr::new(sddl)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .unwrap();
        let mut owner = PSID::default();
        let mut owner_defaulted = BOOL::default();
        let mut dacl = std::ptr::null_mut();
        let mut dacl_present = BOOL::default();
        let mut dacl_defaulted = BOOL::default();
        unsafe {
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted).unwrap();
            GetSecurityDescriptorDacl(
                descriptor,
                &mut dacl_present,
                &mut dacl,
                &mut dacl_defaulted,
            )
            .unwrap();
        }
        let result = dacl_present.as_bool()
            && security_descriptor_has_untrusted_write_access(owner, dacl).unwrap();
        unsafe {
            let _ = LocalFree(HLOCAL(descriptor.0));
        }
        result
    }

    #[test]
    fn autostart_acl_rejects_untrusted_owner_and_user_specific_write_ace() {
        assert!(sddl_has_untrusted_write_access(
            "O:BUD:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGX;;;BU)"
        ));
        assert!(sddl_has_untrusted_write_access(
            "O:SYD:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GW;;;S-1-5-21-1-2-3-1001)"
        ));
        assert!(!sddl_has_untrusted_write_access(
            "O:SYD:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGX;;;BU)"
        ));
    }

    #[test]
    fn windows_program_files_root_is_not_writable_by_low_privilege_groups() {
        let root = program_files_root().unwrap();

        assert!(!path_has_untrusted_write_access(&root).unwrap());
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
    fn registration_verification_and_unregister_precheck_are_behavioral() {
        let cleanup_calls = std::cell::Cell::new(0);
        assert!(verify_registration_with(
            true,
            || (true, true),
            || {
                cleanup_calls.set(99);
                Ok(())
            }
        )
        .is_ok());
        assert_eq!(cleanup_calls.get(), 0);
        assert!(verify_registration_with(
            true,
            || (true, false),
            || {
                cleanup_calls.set(cleanup_calls.get() + 1);
                Ok(())
            }
        )
        .is_err());
        assert_eq!(cleanup_calls.get(), 1);

        let delete_calls = std::cell::Cell::new(0);
        assert!(matches!(
            unregister_with(
                || Ok(false),
                || {
                    delete_calls.set(delete_calls.get() + 1);
                    Ok(())
                }
            ),
            Err(Error::TaskNotFound)
        ));
        assert_eq!(delete_calls.get(), 0);
        assert!(unregister_with(
            || Ok(true),
            || {
                delete_calls.set(delete_calls.get() + 1);
                Ok(())
            }
        )
        .is_ok());
        assert_eq!(delete_calls.get(), 1);

        assert!(registration_matches(true, false, false));
        assert!(registration_matches(true, true, true));
        assert!(!registration_matches(false, true, true));
        assert!(!registration_matches(true, false, true));
    }

    #[test]
    fn unregister_precheck_propagates_query_failure_without_deleting() {
        let delete_calls = std::cell::Cell::new(0);
        let result = unregister_with(
            || Err(Error::UnregisterFailed("Task Scheduler unavailable".into())),
            || {
                delete_calls.set(delete_calls.get() + 1);
                Ok(())
            },
        );

        assert!(matches!(result, Err(Error::UnregisterFailed(_))));
        assert_eq!(delete_calls.get(), 0);
    }

    #[test]
    fn failed_autostart_query_requires_task_file_absence_to_confirm_missing() {
        assert!(matches!(
            task_presence_from_query(false, Ok(false), "query failed".into()),
            Ok(false)
        ));
        assert!(matches!(
            task_presence_from_query(false, Ok(true), "query failed".into()),
            Err(Error::UnregisterFailed(_))
        ));
        assert!(matches!(
            task_presence_from_query(
                false,
                Err("task file metadata denied".into()),
                "query failed".into()
            ),
            Err(Error::UnregisterFailed(_))
        ));
        assert!(matches!(
            task_presence_from_query(true, Err("unused".into()), String::new()),
            Ok(true)
        ));
    }

    #[test]
    fn registration_verification_reports_rollback_failure() {
        let error = verify_registration_with(
            true,
            || (true, false),
            || Err(Error::UnregisterFailed("rollback denied".to_string())),
        )
        .unwrap_err();

        assert!(matches!(error, Error::RegisterFailed(ref detail) if
            detail.contains("정리에도 실패") && detail.contains("rollback denied")));
    }

    #[test]
    fn task_deletion_requires_query_and_task_file_absence() {
        assert!(task_deletion_is_confirmed(false, false));
        assert!(!task_deletion_is_confirmed(true, false));
        assert!(!task_deletion_is_confirmed(false, true));
        assert!(!task_deletion_is_confirmed(true, true));
    }

    #[test]
    fn task_xml_action_parser_keeps_command_and_exact_arguments_separate() {
        let xml = r#"<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger></Triggers>
        <Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
        <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><Enabled>true</Enabled><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings><Actions><Exec>
          <Command>"C:\Program Files\BDO Optimizer\bdo-optimizer-launcher.exe"</Command>
          <Arguments>--minimized</Arguments>
        </Exec></Actions></Task>"#;

        assert_eq!(
            parse_task_action(xml),
            Some((
                r"C:\Program Files\BDO Optimizer\bdo-optimizer-launcher.exe".to_string(),
                "--minimized".to_string()
            ))
        );
    }

    #[test]
    fn task_xml_requires_non_expiring_delayed_restart_contract() {
        let legacy = r#"<Task><Triggers><LogonTrigger /></Triggers>
        <Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
        <Settings><Enabled>true</Enabled><ExecutionTimeLimit>PT72H</ExecutionTimeLimit></Settings><Actions><Exec>
          <Command>C:\app.exe</Command><Arguments>--minimized</Arguments>
        </Exec></Actions></Task>"#;
        let resilient = r#"<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger></Triggers>
        <Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
        <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><Enabled>true</Enabled><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
          <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>
        </Settings><Actions><Exec>
          <Command>C:\app.exe</Command><Arguments>--minimized</Arguments>
        </Exec></Actions></Task>"#;

        assert_eq!(parse_task_action(legacy), None);
        assert_eq!(
            parse_legacy_task_action(legacy),
            Some((r"C:\app.exe".to_string(), MINIMIZED_FLAG.to_string()))
        );
        assert_eq!(
            parse_task_action(resilient),
            Some((r"C:\app.exe".to_string(), MINIMIZED_FLAG.to_string()))
        );
        assert_eq!(parse_legacy_task_action(resilient), None);

        let battery_sensitive = resilient.replace(
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>",
        );
        assert_eq!(parse_task_action(&battery_sensitive), None);
        assert_eq!(
            parse_legacy_task_action(&battery_sensitive),
            Some((r"C:\app.exe".to_string(), MINIMIZED_FLAG.to_string()))
        );

        let parallel = resilient.replace(
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
            "<MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>",
        );
        assert_eq!(parse_task_action(&parallel), None);
        assert_eq!(
            parse_legacy_task_action(&parallel),
            Some((r"C:\app.exe".to_string(), MINIMIZED_FLAG.to_string()))
        );

        let windowed_legacy = legacy.replace("<Arguments>--minimized</Arguments>", "");
        assert_eq!(
            parse_legacy_task_action(&windowed_legacy),
            Some((r"C:\app.exe".to_string(), String::new()))
        );
    }

    #[test]
    fn user_autostart_change_runs_after_inflight_legacy_repair() {
        let lock = std::sync::Arc::new(Mutex::new(()));
        let enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (query_tx, query_rx) = std::sync::mpsc::channel();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();

        std::thread::scope(|scope| {
            let repair_lock = lock.clone();
            let repair_enabled = enabled.clone();
            scope.spawn(move || {
                with_operation_lock(&repair_lock, || {
                    query_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                    repair_enabled.store(true, std::sync::atomic::Ordering::Release);
                });
            });

            query_rx.recv().unwrap();
            let user_lock = lock.clone();
            let user_enabled = enabled.clone();
            scope.spawn(move || {
                with_operation_lock(&user_lock, || {
                    user_enabled.store(false, std::sync::atomic::Ordering::Release);
                });
            });
            continue_tx.send(()).unwrap();
        });

        assert!(!enabled.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn task_xml_rejects_limited_or_disabled_non_logon_tasks() {
        for xml in [
            r#"<Task><Triggers><LogonTrigger /></Triggers><Principals><Principal><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><Enabled>true</Enabled></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec></Actions></Task>"#,
            r#"<Task><Triggers><CalendarTrigger /></Triggers><Principals><Principal><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><Enabled>true</Enabled></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec></Actions></Task>"#,
            r#"<Task><Triggers><LogonTrigger><Enabled>false</Enabled></LogonTrigger></Triggers><Principals><Principal><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><Enabled>true</Enabled></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec></Actions></Task>"#,
            r#"<Task><Triggers><LogonTrigger /></Triggers><Principals><Principal><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><Enabled>false</Enabled></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec></Actions></Task>"#,
        ] {
            assert_eq!(parse_task_action(xml), None);
        }
    }

    #[test]
    fn task_xml_accepts_escaped_quotes_and_rejects_unbalanced_quotes() {
        let valid = r#"<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger></Triggers><Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings><Actions Context="Author"><Exec><Command>&quot;C:\Program Files\app.exe&quot;</Command></Exec></Actions></Task>"#;
        assert_eq!(
            parse_task_action(valid),
            Some((r"C:\Program Files\app.exe".to_string(), String::new()))
        );

        for command in [r#""C:\app.exe"#, r#"C:\app.exe""#, r#""C:\"app.exe""#] {
            let xml = valid.replace("&quot;C:\\Program Files\\app.exe&quot;", command);
            assert_eq!(parse_task_action(&xml), None);
        }
    }

    #[test]
    fn task_xml_rejects_extra_triggers_and_actions() {
        let mixed_trigger = r#"<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger><CalendarTrigger /></Triggers><Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec></Actions></Task>"#;
        let multiple_actions = r#"<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger></Triggers><Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings><Actions><Exec><Command>C:\app.exe</Command></Exec><Exec><Command>C:\other.exe</Command></Exec></Actions></Task>"#;

        assert_eq!(parse_task_action(mixed_trigger), None);
        assert_eq!(parse_task_action(multiple_actions), None);
    }

    #[test]
    fn task_xml_builder_escapes_path_and_emits_resilience_contract() {
        let xml = build_task_xml(
            Path::new(r"C:\Program Files\A&B\bdo-optimizer-launcher.exe"),
            true,
        );

        assert!(xml.contains(r"C:\Program Files\A&amp;B\bdo-optimizer-launcher.exe"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<Delay>PT30S</Delay>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<RestartOnFailure><Interval>PT1M</Interval><Count>3</Count>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<Arguments>--minimized</Arguments>"));
    }

    #[test]
    fn windows_task_scheduler_parses_resilience_contract_without_registration() {
        let xml = build_task_xml(Path::new(r"C:\Windows\System32\notepad.exe"), true);
        let _com = ComInitialization::initialize().unwrap();
        let parsed: windows::core::Result<String> = unsafe {
            (|| {
                let service: ITaskService =
                    CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
                let empty = VARIANT::default();
                service.Connect(&empty, &empty, &empty, &empty)?;
                let definition = service.NewTask(0)?;
                definition.SetXmlText(&BSTR::from(xml.as_str()))?;
                let mut normalized = BSTR::new();
                definition.XmlText(&mut normalized)?;
                Ok(normalized.to_string())
            })()
        };

        let parsed = parsed.unwrap();
        assert!(parsed.contains("<Delay>PT30S</Delay>"));
        assert!(parsed.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(parsed.contains("<RestartOnFailure>"));
        assert!(parsed.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(parsed.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        let triggers = xml_element(&parsed, "Triggers").unwrap();
        let logon_trigger = xml_element(triggers, "LogonTrigger").unwrap();
        let settings = xml_element(&parsed, "Settings").unwrap();
        assert_eq!(
            runtime_contract(logon_trigger, settings),
            Some(RuntimeContract::Current)
        );
    }
}
