// 게임 프로세스 자원 모니터링. CPU%/메모리(MB)/디스크 R·W(KB/s) 측정.
// GPU%/VRAM(MB)는 동일 모듈의 PDH(Performance Data Helper) 카운터로 추가 측정한다.
//
// `Monitor`는 표본 간 상태(이전 CPU 시간, 이전 I/O 카운트, 바운드 PID, PDH 핸들)를 보관한다.
// 동일 PID에 대한 두 번째 호출부터 의미 있는 값을 반환한다. 첫 호출은 차분 부재로 None.

use std::time::Instant;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FALSE, FILETIME, HANDLE};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows::Win32::System::Performance::{
    PdhAddCounterW, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhExpandWildCardPathW, PdhGetCounterInfoW, PdhGetFormattedCounterValue, PdhOpenQueryW,
    PdhRemoveCounter, PDH_COUNTER_INFO_W, PDH_FMT, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    PDH_FMT_LARGE, PDH_REFRESHCOUNTERS,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

// PDH 핸들은 windows 0.58에서 raw isize. 의미 명확화를 위해 alias로 표시한다.
type PdhQuery = isize;
type PdhCounter = isize;
struct NamedPdhCounter {
    handle: PdhCounter,
    instance: String,
}
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessAffinityMask, GetProcessIoCounters, GetProcessTimes, OpenProcess, IO_COUNTERS,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Clone, Default)]
pub struct MonitorSample {
    pub cpu_pct: Option<f64>,
    pub mem_mb: Option<u64>,
    pub gpu_pct: Option<f64>,
    pub vram_mb: Option<u64>,
    pub disk_read_kbs: Option<u64>,
    pub disk_write_kbs: Option<u64>,
    // 시스템 코어별 사용률 (코어 인덱스 순서). 비어 있으면 미수집.
    pub core_usages: Vec<f64>,
    // 게임 프로세스 affinity 마스크 (코어 i가 사용 가능하면 비트 i가 1).
    pub affinity_mask: Option<usize>,
}

pub struct Monitor {
    bound_pid: Option<u32>,
    prev_instant: Option<Instant>,
    prev_cpu_total_100ns: Option<u64>,
    prev_io_read: Option<u64>,
    prev_io_write: Option<u64>,
    cores: u32,
    pdh_query: Option<PdhQuery>,
    pdh_gpu_util: Vec<NamedPdhCounter>,
    pdh_gpu_mem: Vec<NamedPdhCounter>,
    pdh_cpu_cores: Vec<NamedPdhCounter>,
    pdh_retry_after: Option<Instant>,
    pub total_ram_mb: u64,
    pub total_vram_mb: u64,
}

impl Monitor {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);
        Self {
            bound_pid: None,
            prev_instant: None,
            prev_cpu_total_100ns: None,
            prev_io_read: None,
            prev_io_write: None,
            cores,
            pdh_query: None,
            pdh_gpu_util: Vec::new(),
            pdh_gpu_mem: Vec::new(),
            pdh_cpu_cores: Vec::new(),
            pdh_retry_after: None,
            total_ram_mb: unsafe { query_total_ram_mb() },
            total_vram_mb: unsafe { query_total_vram_mb() },
        }
    }

    pub fn rebind(&mut self, pid: Option<u32>) {
        if self.bound_pid != pid {
            self.bound_pid = pid;
            self.prev_instant = None;
            self.prev_cpu_total_100ns = None;
            self.prev_io_read = None;
            self.prev_io_write = None;
            self.close_pdh();
            if let Some(p) = pid {
                unsafe {
                    self.setup_pdh(p);
                }
            }
        }
    }

    fn close_pdh(&mut self) {
        if let Some(q) = self.pdh_query.take() {
            unsafe {
                let _ = PdhCloseQuery(q);
            }
        }
        self.pdh_gpu_util.clear();
        self.pdh_gpu_mem.clear();
        self.pdh_cpu_cores.clear();
        self.pdh_retry_after = None;
    }

    unsafe fn setup_pdh(&mut self, pid: u32) {
        let mut hq: PdhQuery = 0;
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut hq) != 0 {
            return;
        }
        self.pdh_query = Some(hq);

        let util_path = format!(
            r"\GPU Engine(pid_{}_*engtype_3D)\Utilization Percentage",
            pid
        );
        self.pdh_gpu_util = pdh_add_english_wildcard_counters(hq, &util_path);

        let mem_path = format!(r"\GPU Process Memory(pid_{}_*)\Dedicated Usage", pid);
        self.pdh_gpu_mem = pdh_add_english_wildcard_counters(hq, &mem_path);

        // 시스템 코어별 사용률 — PID 독립 wildcard. _Total은 sample 시 필터.
        let cpu_path = r"\Processor Information(*)\% Processor Time";
        self.pdh_cpu_cores = pdh_add_english_wildcard_counters(hq, cpu_path);
        self.pdh_retry_after = (self.pdh_gpu_util.is_empty()
            || self.pdh_gpu_mem.is_empty()
            || self.pdh_cpu_cores.is_empty())
        .then(|| Instant::now() + std::time::Duration::from_secs(5));

        // 더미 collect 1회 — 차분 기반 카운터의 첫 표본 0 회피
        let _ = PdhCollectQueryData(hq);
    }

    pub fn sample(&mut self, pid: u32) -> MonitorSample {
        if self.bound_pid != Some(pid) {
            self.rebind(Some(pid));
        }

        let now = Instant::now();
        if self
            .pdh_retry_after
            .is_some_and(|retry_after| now >= retry_after)
        {
            self.close_pdh();
            unsafe {
                self.setup_pdh(pid);
            }
        }
        let dt = self
            .prev_instant
            .map(|p| (now - p).as_secs_f64())
            .unwrap_or(0.0);
        self.prev_instant = Some(now);

        let mut sample = MonitorSample::default();

        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        let h = match handle {
            Ok(h) => h,
            Err(_) => return sample,
        };

        sample.cpu_pct = unsafe { self.sample_cpu(h, dt) };
        sample.mem_mb = unsafe { sample_mem(h) };
        let (r, w) = unsafe { self.sample_io(h, dt) };
        sample.disk_read_kbs = r;
        sample.disk_write_kbs = w;
        sample.affinity_mask = unsafe { sample_affinity(h) };

        unsafe {
            let _ = CloseHandle(h);
        }

        if let Some(q) = self.pdh_query {
            unsafe {
                let _ = PdhCollectQueryData(q);
            }
            sample.gpu_pct = unsafe { pdh_sum_double(&self.pdh_gpu_util) };
            sample.vram_mb =
                unsafe { pdh_sum_large(&self.pdh_gpu_mem).map(|b| (b / 1024 / 1024) as u64) };
            sample.core_usages = unsafe { pdh_collect_per_core(&self.pdh_cpu_cores) };
        }

        sample
    }

    unsafe fn sample_cpu(&mut self, handle: HANDLE, dt: f64) -> Option<f64> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
        let total = filetime_to_u64(kernel) + filetime_to_u64(user);
        let prev = self.prev_cpu_total_100ns.replace(total);
        match prev {
            Some(p) if dt > 0.0 => {
                let delta_100ns = total.saturating_sub(p) as f64;
                let cpu_seconds = delta_100ns / 1e7;
                let pct = cpu_seconds / dt / self.cores as f64 * 100.0;
                Some(pct.clamp(0.0, 100.0))
            }
            _ => None,
        }
    }

    unsafe fn sample_io(&mut self, handle: HANDLE, dt: f64) -> (Option<u64>, Option<u64>) {
        let mut io = IO_COUNTERS::default();
        if GetProcessIoCounters(handle, &mut io).is_err() {
            return (None, None);
        }
        let r = io.ReadTransferCount;
        let w = io.WriteTransferCount;
        let prev_r = self.prev_io_read.replace(r);
        let prev_w = self.prev_io_write.replace(w);
        let dr = match prev_r {
            Some(p) if dt > 0.0 => {
                let diff = r.saturating_sub(p) as f64;
                Some((diff / dt / 1024.0) as u64)
            }
            _ => None,
        };
        let dw = match prev_w {
            Some(p) if dt > 0.0 => {
                let diff = w.saturating_sub(p) as f64;
                Some((diff / dt / 1024.0) as u64)
            }
            _ => None,
        };
        (dr, dw)
    }
}

// 시스템 전체 물리 RAM 크기(MB). GlobalMemoryStatusEx의 ullTotalPhys 사용.
unsafe fn query_total_ram_mb() -> u64 {
    let mut info = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if GlobalMemoryStatusEx(&mut info).is_err() {
        return 0;
    }
    info.ullTotalPhys / 1024 / 1024
}

// 시스템 내 dedicated VRAM이 가장 큰 GPU의 VRAM 크기(MB).
// DXGI로 어댑터 목록을 조회해 software 어댑터(WARP) 제외, max DedicatedVideoMemory 반환.
unsafe fn query_total_vram_mb() -> u64 {
    let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
        Ok(f) => f,
        Err(_) => return 0,
    };
    let mut best: u64 = 0;
    let mut idx = 0u32;
    loop {
        let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(idx) {
            Ok(a) => a,
            Err(_) => break,
        };
        if let Ok(desc) = adapter.GetDesc1() {
            // software 어댑터(WARP/Basic Render Driver) 제외.
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
                let bytes = desc.DedicatedVideoMemory as u64;
                if bytes > best {
                    best = bytes;
                }
            }
        }
        idx += 1;
    }
    best / 1024 / 1024
}

unsafe fn sample_mem(handle: HANDLE) -> Option<u64> {
    let mut info = PROCESS_MEMORY_COUNTERS::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    GetProcessMemoryInfo(handle, &mut info as *mut _, size).ok()?;
    Some(info.WorkingSetSize as u64 / 1024 / 1024)
}

unsafe fn sample_affinity(handle: HANDLE) -> Option<usize> {
    let mut proc_mask: usize = 0;
    let mut sys_mask: usize = 0;
    GetProcessAffinityMask(handle, &mut proc_mask, &mut sys_mask).ok()?;
    Some(proc_mask)
}

// "0,0" / "0,1" / ... 형식의 instance name만 (group, cpu) 튜플로 파싱한다.
// PDH는 "_Total"뿐 아니라 "0,_total" 같은 그룹 합계도 반환하므로 숫자 쌍만 코어로 인정한다.
fn parse_instance(name: &str) -> Option<(u32, u32)> {
    let (group, cpu) = name.split_once(',')?;
    if group.is_empty()
        || cpu.is_empty()
        || !group.bytes().all(|b| b.is_ascii_digit())
        || !cpu.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((group.parse().ok()?, cpu.parse().ok()?))
}

unsafe fn pwstr_to_string(p: PWSTR) -> String {
    if p.0.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *p.0.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(p.0, len);
    String::from_utf16_lossy(slice)
}

const _: () = assert!(std::mem::align_of::<PDH_COUNTER_INFO_W>() <= std::mem::align_of::<u64>());

fn split_multi_sz(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|value| *value == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn counter_instance(path: &str) -> Option<String> {
    let end = path.rfind(")\\")?;
    let start = path[..end].rfind('(')?;
    Some(path[start + 1..end].to_string())
}

unsafe fn localized_wildcard_path(counter: PdhCounter) -> Option<String> {
    let mut byte_len = 0u32;
    let _ = PdhGetCounterInfoW(counter, false, &mut byte_len, None);
    if byte_len == 0 {
        return None;
    }
    let mut buffer = vec![0u64; (byte_len as usize).div_ceil(std::mem::size_of::<u64>())];
    let info = buffer.as_mut_ptr() as *mut PDH_COUNTER_INFO_W;
    let status = PdhGetCounterInfoW(counter, false, &mut byte_len, Some(info));
    if status != 0 {
        tracing::warn!(status, "PDH counter info lookup failed");
        return None;
    }
    Some(pwstr_to_string((*info).szFullPath))
}

unsafe fn expand_localized_wildcard(path: &str) -> Vec<String> {
    let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut char_len = 0u32;
    let _ = PdhExpandWildCardPathW(
        PCWSTR::null(),
        PCWSTR(path_w.as_ptr()),
        PWSTR::null(),
        &mut char_len,
        PDH_REFRESHCOUNTERS,
    );
    if char_len == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u16; char_len as usize];
    let status = PdhExpandWildCardPathW(
        PCWSTR::null(),
        PCWSTR(path_w.as_ptr()),
        PWSTR(buffer.as_mut_ptr()),
        &mut char_len,
        PDH_REFRESHCOUNTERS,
    );
    if status != 0 {
        tracing::warn!(status, "PDH wildcard expansion failed");
        return Vec::new();
    }
    split_multi_sz(&buffer[..char_len as usize])
}

unsafe fn pdh_add_english_wildcard_counters(
    query: PdhQuery,
    english_path: &str,
) -> Vec<NamedPdhCounter> {
    let english_w: Vec<u16> = english_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut temporary = 0;
    let status = PdhAddEnglishCounterW(query, PCWSTR(english_w.as_ptr()), 0, &mut temporary);
    if status != 0 {
        tracing::warn!(
            status,
            english_path,
            "PDH English counter registration failed"
        );
        return Vec::new();
    }
    let localized = localized_wildcard_path(temporary);
    let _ = PdhRemoveCounter(temporary);
    let Some(localized) = localized else {
        return Vec::new();
    };

    expand_localized_wildcard(&localized)
        .into_iter()
        .filter_map(|path| {
            let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut handle = 0;
            let status = PdhAddCounterW(query, PCWSTR(path_w.as_ptr()), 0, &mut handle);
            if status != 0 {
                tracing::warn!(status, path, "PDH localized counter registration failed");
                return None;
            }
            Some(NamedPdhCounter {
                handle,
                instance: counter_instance(&path).unwrap_or_default(),
            })
        })
        .collect()
}

unsafe fn pdh_scalar(counter: PdhCounter, format: PDH_FMT) -> Option<PDH_FMT_COUNTERVALUE> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = PdhGetFormattedCounterValue(counter, format, None, &mut value);
    (status == 0 && value.CStatus <= 1).then_some(value)
}

unsafe fn pdh_collect_per_core(counters: &[NamedPdhCounter]) -> Vec<f64> {
    let mut pairs = counters
        .iter()
        .filter_map(|counter| {
            let key = parse_instance(&counter.instance)?;
            let value = pdh_scalar(counter.handle, PDH_FMT_DOUBLE)?;
            Some((key, value.Anonymous.doubleValue))
        })
        .collect::<Vec<_>>();
    pairs.sort_by_key(|pair| pair.0);
    pairs.into_iter().map(|(_, value)| value).collect()
}

fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

unsafe fn pdh_sum_double(counters: &[NamedPdhCounter]) -> Option<f64> {
    let values = counters
        .iter()
        .filter_map(|counter| pdh_scalar(counter.handle, PDH_FMT_DOUBLE))
        .map(|value| value.Anonymous.doubleValue)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.into_iter().sum())
}

unsafe fn pdh_sum_large(counters: &[NamedPdhCounter]) -> Option<i64> {
    let values = counters
        .iter()
        .filter_map(|counter| pdh_scalar(counter.handle, PDH_FMT_LARGE))
        .map(|value| value.Anonymous.largeValue)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.into_iter().sum())
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.close_pdh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_instance_splits_group_and_cpu() {
        assert_eq!(parse_instance("0,0"), Some((0, 0)));
        assert_eq!(parse_instance("0,5"), Some((0, 5)));
        assert_eq!(parse_instance("1,12"), Some((1, 12)));
    }

    #[test]
    fn parse_instance_rejects_total_and_partial_instances() {
        assert_eq!(parse_instance("_Total"), None);
        assert_eq!(parse_instance("0,_total"), None);
        assert_eq!(parse_instance(""), None);
        assert_eq!(parse_instance("3"), None);
    }

    #[test]
    fn multi_sz_and_localized_counter_instance_are_parsed() {
        let buffer = [b'a' as u16, 0, b'b' as u16, b'c' as u16, 0, 0];
        assert_eq!(split_multi_sz(&buffer), vec!["a", "bc"]);
        assert_eq!(
            counter_instance(r"\프로세서 정보(0,7)\% 프로세서 시간").as_deref(),
            Some("0,7")
        );
    }

    #[test]
    fn english_cpu_wildcard_expands_on_current_windows_locale() {
        unsafe {
            let mut query = 0;
            assert_eq!(PdhOpenQueryW(PCWSTR::null(), 0, &mut query), 0);
            let counters = pdh_add_english_wildcard_counters(
                query,
                r"\Processor Information(*)\% Processor Time",
            );
            let _ = PdhCloseQuery(query);
            assert!(!counters.is_empty());
            assert!(counters
                .iter()
                .any(|counter| parse_instance(&counter.instance).is_some()));
        }
    }
}
