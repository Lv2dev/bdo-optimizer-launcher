use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FALSE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformation, GetLogicalProcessorInformationEx, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::{
    GetPriorityClass, OpenProcess, QueryFullProcessImageNameW, SetPriorityClass,
    SetProcessAffinityMask, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_CREATION_FLAGS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

// M66b: thiserror enum. 호출처 Display 메시지를 기존 String과 동일하게 유지.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("프로세스 열기 실패 (PID {pid}): {source}")]
    OpenProcess {
        pid: u32,
        source: windows::core::Error,
    },
    #[error("PID {0} 검증 실패. 게임 프로세스가 종료된 것으로 보입니다.")]
    ImageMismatch(u32),
    #[error("Priority 설정 실패: {0}")]
    SetPriority(windows::core::Error),
    #[error("Affinity 설정 실패: {0}")]
    SetAffinity(windows::core::Error),
}

pub struct CpuInfo {
    pub physical_cores: u32,
    pub logical_threads: u32,
    /// Alder Lake+ Intel의 P-core 비트마스크. 비-hybrid CPU에서는 0.
    pub p_core_mask: usize,
    /// Alder Lake+ Intel의 E-core 비트마스크. 비-hybrid CPU에서는 0.
    pub e_core_mask: usize,
    /// EfficiencyClass가 둘 이상이면 true (Alder Lake+ Intel).
    pub has_hybrid: bool,
}

pub fn get_cpu_info() -> CpuInfo {
    let fallback_logical = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    // 1) P/E-core 인식 시도 (Win10 1809+ GetLogicalProcessorInformationEx + EfficiencyClass).
    //    실패 시 (구형 OS 등) 기존 GetLogicalProcessorInformation으로 fallback.
    if let Some((p_mask, e_mask, phys, logical)) = unsafe { enumerate_cores_ex() } {
        let has_hybrid = e_mask != 0;
        return CpuInfo {
            physical_cores: phys.max(1),
            logical_threads: logical.max(fallback_logical),
            p_core_mask: p_mask,
            e_core_mask: e_mask,
            has_hybrid,
        };
    }

    // 2) Fallback — P/E 정보 없음, 기존 동작 유지.
    let physical_cores = count_physical_cores().unwrap_or((fallback_logical / 2).max(1));
    CpuInfo {
        physical_cores,
        logical_threads: fallback_logical,
        p_core_mask: 0,
        e_core_mask: 0,
        has_hybrid: false,
    }
}

fn count_physical_cores() -> Option<u32> {
    unsafe {
        let elem = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>() as u32;
        let mut len: u32 = 0;
        // 첫 번째 호출: 필요한 버퍼 크기를 len에 채움 (ERROR_INSUFFICIENT_BUFFER 반환됨)
        let _ = GetLogicalProcessorInformation(None, &mut len);
        if len == 0 {
            return None;
        }
        let count = (len / elem) as usize;
        let mut buf: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> =
            (0..count).map(|_| std::mem::zeroed()).collect();
        GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut len).ok()?;
        let actual = (len / elem) as usize;
        Some(
            buf[..actual]
                .iter()
                .filter(|e| e.Relationship == RelationProcessorCore)
                .count() as u32,
        )
    }
}

/// GetLogicalProcessorInformationEx(RelationProcessorCore)로 P/E-core mask 수집.
/// 반환: (p_core_mask, e_core_mask, physical_cores, logical_threads).
/// EfficiencyClass 최댓값을 P-core로, 그 외를 E-core로 분류 (비-hybrid CPU는 모두 같은 class라 e_mask=0).
/// 가변 길이 구조체이므로 byte 단위 offset + Size 필드로 순회.
unsafe fn enumerate_cores_ex() -> Option<(usize, usize, u32, u32)> {
    let mut len: u32 = 0;
    // 첫 호출: ERROR_INSUFFICIENT_BUFFER 예상 (성공하지 않음 OK).
    let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len);
    if len == 0 {
        return None;
    }

    let mut buf: Vec<u8> = vec![0u8; len as usize];
    GetLogicalProcessorInformationEx(
        RelationProcessorCore,
        Some(buf.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX),
        &mut len,
    )
    .ok()?;

    // 단일 pass로 (EfficiencyClass, mask) 쌍을 수집. 분류는 safe fn `classify_cores`에 위임.
    let mut entries: Vec<(u8, u64)> = Vec::new();
    let mut offset = 0usize;
    while offset + std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>() <= len as usize {
        let entry = &*(buf.as_ptr().add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX);
        let entry_size = entry.Size as usize;
        if entry_size == 0 || offset + entry_size > len as usize {
            break;
        }
        if entry.Relationship == RelationProcessorCore {
            let proc = &entry.Anonymous.Processor;
            if proc.GroupCount >= 1 {
                // GroupCount > 1 (Processor Group 다중)은 일반 게이밍 PC 밖이라 첫 그룹만 사용.
                let mask = proc.GroupMask[0].Mask as u64;
                entries.push((proc.EfficiencyClass, mask));
            }
        }
        offset += entry_size;
    }

    let (p_core_mask, e_core_mask, physical_cores, logical_threads) = classify_cores(&entries);
    if physical_cores == 0 {
        return None;
    }
    Some((
        p_core_mask as usize,
        e_core_mask as usize,
        physical_cores,
        logical_threads,
    ))
}

/// `(EfficiencyClass, mask)` 쌍 리스트를 P/E-core mask + 코어 수로 분류한다.
/// EfficiencyClass 최댓값을 P-core, 그 외를 E-core로 본다.
/// 비-hybrid CPU(모든 코어 같은 class)는 모두 P-core가 되어 `e_core_mask = 0` (자연 fallback).
/// 빈 입력에 대해 `(0, 0, 0, 0)` 반환.
/// 반환: `(p_core_mask, e_core_mask, physical_cores, logical_threads)`.
pub(crate) fn classify_cores(entries: &[(u8, u64)]) -> (u64, u64, u32, u32) {
    let max_class = match entries.iter().map(|(c, _)| *c).max() {
        Some(c) => c,
        None => return (0, 0, 0, 0),
    };
    let mut p_core_mask: u64 = 0;
    let mut e_core_mask: u64 = 0;
    let mut physical_cores: u32 = 0;
    let mut logical_threads: u32 = 0;
    for &(class, mask) in entries {
        physical_cores += 1;
        logical_threads += mask.count_ones();
        if class == max_class {
            p_core_mask |= mask;
        } else {
            e_core_mask |= mask;
        }
    }
    (p_core_mask, e_core_mask, physical_cores, logical_threads)
}

// 주의: usize 비트 시프트를 사용하므로 논리 스레드가 usize 비트 수(64) 이상이거나
// Windows Processor Group이 적용되는 고사양 워크스테이션에서는 동작을 별도 검증해야 한다.
// 64코어 이상에서는 debug_assert가 알리고, release에서는 64로 클램프해 wrap-around를 막는다.
//
// Alder Lake+ Intel(hybrid CPU): P-core 전체 thread만 활성, E-core 차단(검은사막은 E-core 끄는 게 성능 ↑).
pub fn calc_high_affinity(info: &CpuInfo) -> usize {
    if info.has_hybrid && info.p_core_mask != 0 {
        return info.p_core_mask;
    }
    debug_assert!(
        info.logical_threads <= 64,
        "64코어 초과 환경에서는 Processor Group을 고려한 별도 affinity 계산이 필요합니다."
    );
    if info.physical_cores == 0 {
        return 0;
    }
    let phys = (info.physical_cores as usize).min(64);
    let smt = (info.logical_threads / info.physical_cores).max(1) as usize;
    let mut mask: usize = 0;
    for i in 0..phys {
        let bit = (i * smt).min(63);
        mask |= 1 << bit;
    }
    mask
}

// 일반 모드: 모든 논리 스레드 사용 가능. 64코어 이상은 64로 클램프된 전체 비트마스크.
pub fn calc_normal_affinity(info: &CpuInfo) -> usize {
    debug_assert!(
        info.logical_threads <= 64,
        "64코어 초과 환경에서는 Processor Group을 고려한 별도 affinity 계산이 필요합니다."
    );
    let n = (info.logical_threads as usize).min(64);
    if n == 0 {
        return 0;
    }
    if n >= usize::BITS as usize {
        usize::MAX
    } else {
        (1usize << n) - 1
    }
}

// Alder Lake+ Intel(hybrid CPU): E-core 전체에 게임 백그라운드 실행 (P-core 차단).
// 비-hybrid CPU: 마지막 1~2 코어로 idle 수준 실행.
pub fn calc_low_power_affinity(info: &CpuInfo) -> usize {
    if info.has_hybrid && info.e_core_mask != 0 {
        return info.e_core_mask;
    }
    debug_assert!(
        info.logical_threads <= 64,
        "64코어 초과 환경에서는 Processor Group을 고려한 별도 affinity 계산이 필요합니다."
    );
    let n = (info.logical_threads as usize).min(64);
    if n < 2 {
        return 1;
    }
    let smt = (info.logical_threads / info.physical_cores.max(1)) as usize;
    if smt >= 2 {
        (1 << (n - 2)) | (1 << (n - 1))
    } else {
        1 << (n - 1)
    }
}

// PID 캐시: 동일 게임 프로세스라면 toolhelp 풀스캔을 생략한다.
// 캐시된 PID는 OpenProcess + QueryFullProcessImageNameW로 검증한 뒤에만 재사용.
static CACHED_PID: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

pub fn find_process_id(exe_name: &str) -> Option<u32> {
    // 1) 캐시 검증: 살아있고 이미지명이 일치하면 그대로 반환.
    //    poison된 mutex는 graceful fallback(풀스캔으로 진행).
    let cached = CACHED_PID.lock().ok().and_then(|g| *g);
    if let Some(pid) = cached {
        if verify_pid_image(pid, exe_name) {
            return Some(pid);
        }
    }

    // 2) 풀스캔
    let found = scan_for_pid(exe_name);
    if let Ok(mut guard) = CACHED_PID.lock() {
        *guard = found;
    }
    found
}

fn verify_pid_image(pid: u32, exe_name: &str) -> bool {
    unsafe {
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let name = query_image_name(handle);
        let _ = CloseHandle(handle);
        name.as_deref()
            .map(|n| n.eq_ignore_ascii_case(exe_name))
            .unwrap_or(false)
    }
}

fn scan_for_pid(exe_name: &str) -> Option<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry as *mut PROCESSENTRY32W).is_err() {
            let _ = CloseHandle(snapshot);
            return None;
        }
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if exe.eq_ignore_ascii_case(exe_name) {
                let pid = entry.th32ProcessID;
                let _ = CloseHandle(snapshot);
                return Some(pid);
            }
            if Process32NextW(snapshot, &mut entry as *mut PROCESSENTRY32W).is_err() {
                break;
            }
        }
        let _ = CloseHandle(snapshot);
        None
    }
}

/// BlackDesert64.exe의 현재 Priority Class를 읽어 적용된 모드를 반환.
/// HIGH → High, NORMAL → Normal, IDLE → LowPower, 그 외 → None.
/// M78: `Option<&'static str>` → `Option<OptimizeMode>`로 격상. 호출처는
/// `ui::control::mode_label`로 label 변환.
pub fn query_current_mode(pid: u32) -> Option<super::schedule::OptimizeMode> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid).ok()?;
        let priority = GetPriorityClass(handle);
        let _ = CloseHandle(handle);
        priority_class_to_mode(priority)
    }
}

// pure 분류 fn (priority class → OptimizeMode). unsafe 영역 밖에서 단위 테스트로 고정.
pub(super) fn priority_class_to_mode(priority: u32) -> Option<super::schedule::OptimizeMode> {
    use super::schedule::OptimizeMode;
    if priority == HIGH_PRIORITY_CLASS.0 {
        Some(OptimizeMode::High)
    } else if priority == NORMAL_PRIORITY_CLASS.0 {
        Some(OptimizeMode::Normal)
    } else if priority == IDLE_PRIORITY_CLASS.0 {
        Some(OptimizeMode::LowPower)
    } else {
        None
    }
}

pub fn apply_optimization(
    pid: u32,
    affinity: usize,
    priority: PROCESS_CREATION_FLAGS,
) -> Result<(), Error> {
    unsafe {
        // PROCESS_QUERY_LIMITED_INFORMATION을 함께 요청해 이미지명 재확인 가능.
        // PID 재사용 TOCTOU로 무관한 elevated 프로세스를 조작하지 않도록 방어한다.
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION
                | windows::Win32::System::Threading::PROCESS_SET_INFORMATION,
            FALSE,
            pid,
        )
        .map_err(|source| Error::OpenProcess { pid, source })?;

        // 이미지 파일명 재확인
        let image_name = query_image_name(handle);
        let matches = image_name
            .as_deref()
            .map(|n| n.eq_ignore_ascii_case("BlackDesert64.exe"))
            .unwrap_or(false);
        if !matches {
            let _ = CloseHandle(handle);
            return Err(Error::ImageMismatch(pid));
        }

        if let Err(e) = SetPriorityClass(handle, priority) {
            let _ = CloseHandle(handle);
            return Err(Error::SetPriority(e));
        }
        if let Err(e) = SetProcessAffinityMask(handle, affinity) {
            let _ = CloseHandle(handle);
            return Err(Error::SetAffinity(e));
        }
        let _ = CloseHandle(handle);
        Ok(())
    }
}

unsafe fn query_image_name(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
    let mut buf: [u16; 260] = [0; 260];
    let mut size: u32 = buf.len() as u32;
    QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        PWSTR(buf.as_mut_ptr()),
        &mut size,
    )
    .ok()?;
    let full = String::from_utf16_lossy(&buf[..size as usize]);
    std::path::Path::new(&full)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::schedule::OptimizeMode;
    use super::*;

    #[test]
    fn priority_class_to_mode_maps_known_values() {
        assert_eq!(
            priority_class_to_mode(HIGH_PRIORITY_CLASS.0),
            Some(OptimizeMode::High)
        );
        assert_eq!(
            priority_class_to_mode(NORMAL_PRIORITY_CLASS.0),
            Some(OptimizeMode::Normal)
        );
        assert_eq!(
            priority_class_to_mode(IDLE_PRIORITY_CLASS.0),
            Some(OptimizeMode::LowPower)
        );
        assert_eq!(priority_class_to_mode(0), None);
        // 알 수 없는 값은 None.
        assert_eq!(priority_class_to_mode(0x4000), None);
    }

    fn info(physical: u32, logical: u32) -> CpuInfo {
        CpuInfo {
            physical_cores: physical,
            logical_threads: logical,
            p_core_mask: 0,
            e_core_mask: 0,
            has_hybrid: false,
        }
    }

    fn hybrid_info(p_mask: usize, e_mask: usize, physical: u32, logical: u32) -> CpuInfo {
        CpuInfo {
            physical_cores: physical,
            logical_threads: logical,
            p_core_mask: p_mask,
            e_core_mask: e_mask,
            has_hybrid: true,
        }
    }

    #[test]
    fn high_affinity_smt2_uses_even_bits() {
        // 4 physical / 8 logical (SMT 2): 비트 0, 2, 4, 6 = 0x55
        assert_eq!(calc_high_affinity(&info(4, 8)), 0b01010101);
    }

    #[test]
    fn high_affinity_smt1_uses_consecutive_bits() {
        // 8 physical / 8 logical (SMT 1): 비트 0..7 = 0xFF
        assert_eq!(calc_high_affinity(&info(8, 8)), 0xFF);
    }

    #[test]
    fn high_affinity_zero_physical_returns_zero() {
        assert_eq!(calc_high_affinity(&info(0, 8)), 0);
    }

    #[test]
    fn high_affinity_six_physical_smt2() {
        // 6 phys / 12 log: 비트 0, 2, 4, 6, 8, 10 = 0x555
        assert_eq!(calc_high_affinity(&info(6, 12)), 0b010101010101);
    }

    #[test]
    fn normal_affinity_full_bitmask() {
        assert_eq!(calc_normal_affinity(&info(4, 8)), 0xFF);
        assert_eq!(calc_normal_affinity(&info(8, 16)), 0xFFFF);
        assert_eq!(calc_normal_affinity(&info(1, 1)), 0b1);
    }

    #[test]
    fn normal_affinity_zero_logical() {
        assert_eq!(calc_normal_affinity(&info(0, 0)), 0);
    }

    #[test]
    fn normal_affinity_clamps_at_64() {
        // 64 logical: 모든 비트 set = usize::MAX (64-bit usize 기준)
        assert_eq!(calc_normal_affinity(&info(32, 64)), usize::MAX);
    }

    #[test]
    fn low_power_smt2_uses_last_two_bits() {
        // 4 phys / 8 log: 비트 6, 7 = 0xC0
        assert_eq!(calc_low_power_affinity(&info(4, 8)), 0b11000000);
    }

    #[test]
    fn low_power_smt1_uses_last_bit() {
        // 8 phys / 8 log: 비트 7만 = 0x80
        assert_eq!(calc_low_power_affinity(&info(8, 8)), 0b10000000);
    }

    #[test]
    fn low_power_single_logical_returns_bit_zero() {
        assert_eq!(calc_low_power_affinity(&info(1, 1)), 1);
    }

    #[test]
    fn low_power_two_logical_smt2() {
        // 1 phys / 2 log (SMT 2): 비트 0, 1 = 0x3
        assert_eq!(calc_low_power_affinity(&info(1, 2)), 0b11);
    }

    #[test]
    fn low_power_zero_logical_treated_as_minimal() {
        // logical 0이면 n=0, n<2 분기로 1 반환
        assert_eq!(calc_low_power_affinity(&info(0, 0)), 1);
    }

    // --- M61 P/E-core (Alder Lake+) 시나리오 ---

    #[test]
    fn hybrid_high_returns_p_core_mask_only() {
        // 13700K simulation: P-core 0~15비트(8P × SMT 2), E-core 16~23비트(8E × SMT 1)
        let p_mask: usize = 0xFFFF; // 비트 0~15
        let e_mask: usize = 0xFF_0000; // 비트 16~23
        let i = hybrid_info(p_mask, e_mask, 16, 24);
        assert_eq!(calc_high_affinity(&i), p_mask);
    }

    #[test]
    fn hybrid_low_power_returns_e_core_mask_only() {
        // 13700K: 고성능과 정반대로 E-core만
        let p_mask: usize = 0xFFFF;
        let e_mask: usize = 0xFF_0000;
        let i = hybrid_info(p_mask, e_mask, 16, 24);
        assert_eq!(calc_low_power_affinity(&i), e_mask);
    }

    #[test]
    fn hybrid_normal_includes_all_threads() {
        // 일반 모드는 전체 코어 (hybrid 무관, 기존 로직)
        let i = hybrid_info(0xFFFF, 0xFF_0000, 16, 24);
        // 24 logical threads = 0xFFFFFF (24비트 all set)
        assert_eq!(calc_normal_affinity(&i), 0xFF_FFFF);
    }

    #[test]
    fn hybrid_14900k_simulation() {
        // 14900K: 8P (SMT 2 = 16 thread) + 16E (SMT 1 = 16 thread) = 32 logical / 24 physical
        let p_mask: usize = 0xFFFF; // 비트 0~15
        let e_mask: usize = 0xFFFF_0000; // 비트 16~31
        let i = hybrid_info(p_mask, e_mask, 24, 32);
        assert_eq!(calc_high_affinity(&i), p_mask);
        assert_eq!(calc_low_power_affinity(&i), e_mask);
        assert_eq!(calc_normal_affinity(&i), 0xFFFF_FFFF);
    }

    #[test]
    fn non_hybrid_ryzen_uses_legacy_logic() {
        // Ryzen 7800X3D simulation: 8P + SMT 2 = 16 logical, hybrid 아님
        // 고성능 = 짝수 비트 (0, 2, 4, 6, 8, 10, 12, 14) = 0x5555
        let i = info(8, 16);
        assert_eq!(calc_high_affinity(&i), 0x5555);
        // 저전력 = 마지막 2비트 (14, 15) = 0xC000
        assert_eq!(calc_low_power_affinity(&i), 0xC000);
        assert_eq!(calc_normal_affinity(&i), 0xFFFF);
    }

    #[test]
    fn hybrid_with_zero_p_mask_falls_back_to_legacy() {
        // 비정상 상태: has_hybrid=true이지만 p_mask=0 → fallback (안전망)
        let i = hybrid_info(0, 0xFF_0000, 16, 24);
        // p_mask=0이라 hybrid 분기 회피, 기존 로직 (physical=16, smt=1) → 0~15 = 0xFFFF
        assert_eq!(calc_high_affinity(&i), 0xFFFF);
    }

    #[test]
    fn hybrid_with_zero_e_mask_falls_back_to_legacy() {
        // 비정상 상태: has_hybrid=true이지만 e_mask=0 → fallback
        let i = hybrid_info(0xFFFF, 0, 16, 24);
        // e_mask=0이라 hybrid 분기 회피, 기존 low_power 로직 (n=24, smt=1) → 비트 23만 = 0x800000
        assert_eq!(calc_low_power_affinity(&i), 0x800000);
    }

    // --- M67 classify_cores 단위 테스트 ---

    #[test]
    fn classify_cores_empty_returns_zero() {
        assert_eq!(classify_cores(&[]), (0, 0, 0, 0));
    }

    #[test]
    fn classify_cores_non_hybrid_all_become_p() {
        // 모든 코어 class=0 (Ryzen / 구형 Intel)
        let entries = vec![(0, 0b0011), (0, 0b1100)];
        // P-mask = 모두 합 = 0xF, E-mask = 0 (자연 fallback)
        // physical = 2, logical = 4
        assert_eq!(classify_cores(&entries), (0xF, 0, 2, 4));
    }

    #[test]
    fn classify_cores_alder_lake_13700k() {
        // 13700K 시뮬레이션: 8 P-core(class=1, SMT 2 → 16 thread) + 8 E-core(class=0, SMT 1 → 8 thread)
        let mut entries = Vec::new();
        for i in 0..8 {
            // P-core: 2비트씩 (0-1, 2-3, ..., 14-15)
            let mask = 0b11u64 << (i * 2);
            entries.push((1u8, mask));
        }
        for i in 0..8 {
            // E-core: 1비트씩 (16, 17, ..., 23)
            let mask = 1u64 << (16 + i);
            entries.push((0u8, mask));
        }
        let (p, e, phys, log) = classify_cores(&entries);
        assert_eq!(p, 0xFFFF, "P-core mask = 비트 0~15");
        assert_eq!(e, 0xFF_0000, "E-core mask = 비트 16~23");
        assert_eq!(phys, 16);
        assert_eq!(log, 24);
    }

    #[test]
    fn classify_cores_three_class_only_top_is_p() {
        // 가상 시나리오: 3-class CPU (Lunar Lake 등 향후). class=2가 P, 0/1 모두 E.
        let entries = vec![(2u8, 0xFF00), (1u8, 0x00F0), (0u8, 0x000F)];
        let (p, e, phys, log) = classify_cores(&entries);
        assert_eq!(p, 0xFF00, "최상위 class만 P-core");
        assert_eq!(e, 0x00FF, "나머지는 모두 E-core mask로 합산");
        assert_eq!(phys, 3);
        assert_eq!(log, 8 + 4 + 4);
    }

    #[test]
    fn classify_cores_single_core() {
        // 1코어 시스템 (가상 / 가상화)
        assert_eq!(classify_cores(&[(0, 0b1)]), (0b1, 0, 1, 1));
    }

    #[test]
    fn classify_cores_overlapping_masks_or_accumulate() {
        // 동일 class 내 여러 entry의 mask가 OR로 누적
        let entries = vec![(1, 0b0001), (1, 0b0010), (1, 0b0100)];
        assert_eq!(classify_cores(&entries), (0b0111, 0, 3, 3));
    }

    #[test]
    fn classify_cores_14900k_simulation() {
        // 14900K: 8 P-core(class=1, SMT 2 → 16 thread) + 16 E-core(class=0, SMT 1 → 16 thread)
        let mut entries = Vec::new();
        for i in 0..8 {
            entries.push((1u8, 0b11u64 << (i * 2)));
        }
        for i in 0..16 {
            entries.push((0u8, 1u64 << (16 + i)));
        }
        let (p, e, phys, log) = classify_cores(&entries);
        assert_eq!(p, 0xFFFF);
        assert_eq!(e, 0xFFFF_0000);
        assert_eq!(phys, 24);
        assert_eq!(log, 32);
    }
}
