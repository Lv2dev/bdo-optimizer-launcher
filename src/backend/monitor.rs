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
    PdhAddCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_FMT_LARGE,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

// PDH 핸들은 windows 0.58에서 raw isize. 의미 명확화를 위해 alias로 표시한다.
type PdhQuery = isize;
type PdhCounter = isize;
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
    pdh_gpu_util: Option<PdhCounter>,
    pdh_gpu_mem: Option<PdhCounter>,
    pdh_cpu_cores: Option<PdhCounter>,
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
            pdh_gpu_util: None,
            pdh_gpu_mem: None,
            pdh_cpu_cores: None,
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
        self.pdh_gpu_util = None;
        self.pdh_gpu_mem = None;
        self.pdh_cpu_cores = None;
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
        if let Some(h) = pdh_add_counter(hq, &util_path) {
            self.pdh_gpu_util = Some(h);
        }

        let mem_path = format!(r"\GPU Process Memory(pid_{}_*)\Dedicated Usage", pid);
        if let Some(h) = pdh_add_counter(hq, &mem_path) {
            self.pdh_gpu_mem = Some(h);
        }

        // 시스템 코어별 사용률 — PID 독립 wildcard. _Total은 sample 시 필터.
        let cpu_path = r"\Processor Information(*)\% Processor Time";
        if let Some(h) = pdh_add_counter(hq, cpu_path) {
            self.pdh_cpu_cores = Some(h);
        }

        // 더미 collect 1회 — 차분 기반 카운터의 첫 표본 0 회피
        let _ = PdhCollectQueryData(hq);
    }

    pub fn sample(&mut self, pid: u32) -> MonitorSample {
        if self.bound_pid != Some(pid) {
            self.rebind(Some(pid));
        }

        let now = Instant::now();
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
            sample.gpu_pct = self.pdh_gpu_util.and_then(|c| unsafe { pdh_sum_double(c) });
            sample.vram_mb = self
                .pdh_gpu_mem
                .and_then(|c| unsafe { pdh_sum_large(c).map(|b| (b / 1024 / 1024) as u64) });
            sample.core_usages = self
                .pdh_cpu_cores
                .map(|c| unsafe { pdh_collect_per_core(c) })
                .unwrap_or_default();
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

// "0,0" / "0,1" / ... 형식의 instance name을 (group, cpu) 튜플로 파싱한다.
fn parse_instance(name: &str) -> (u32, u32) {
    let mut parts = name.split(',');
    let g = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let c = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (g, c)
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

// `\Processor Information(*)\% Processor Time`에서 코어별 사용률을 수집한다.
// instance name이 "_Total"이면 제외, "group,cpu" 파싱 후 정렬된 코어 인덱스 순서로 반환.
unsafe fn pdh_collect_per_core(counter: PdhCounter) -> Vec<f64> {
    let mut size = 0u32;
    let mut count = 0u32;
    let _ = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
    if size == 0 || count == 0 {
        return Vec::new();
    }
    let mut buf: Vec<u8> = vec![0; size as usize];
    let res = PdhGetFormattedCounterArrayW(
        counter,
        PDH_FMT_DOUBLE,
        &mut size,
        &mut count,
        Some(buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
    );
    if res != 0 {
        return Vec::new();
    }
    let items_ptr = buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W;
    let items = std::slice::from_raw_parts(items_ptr, count as usize);
    let mut pairs: Vec<((u32, u32), f64)> = Vec::with_capacity(count as usize);
    for item in items {
        let name = pwstr_to_string(item.szName);
        if name == "_Total" {
            continue;
        }
        let key = parse_instance(&name);
        pairs.push((key, item.FmtValue.Anonymous.doubleValue));
    }
    pairs.sort_by_key(|a| a.0);
    pairs.into_iter().map(|(_, v)| v).collect()
}

fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

fn pdh_add_counter(hq: PdhQuery, path: &str) -> Option<PdhCounter> {
    let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut h: PdhCounter = 0;
    let res = unsafe { PdhAddCounterW(hq, PCWSTR(path_w.as_ptr()), 0, &mut h) };
    if res == 0 {
        Some(h)
    } else {
        None
    }
}

unsafe fn pdh_sum_double(counter: PdhCounter) -> Option<f64> {
    let items = pdh_collect_items(counter, PDH_FMT_DOUBLE)?;
    let sum: f64 = items.iter().map(|i| i.FmtValue.Anonymous.doubleValue).sum();
    Some(sum)
}

unsafe fn pdh_sum_large(counter: PdhCounter) -> Option<i64> {
    let items = pdh_collect_items(counter, PDH_FMT_LARGE)?;
    let sum: i64 = items.iter().map(|i| i.FmtValue.Anonymous.largeValue).sum();
    Some(sum)
}

unsafe fn pdh_collect_items(
    counter: PdhCounter,
    fmt: PDH_FMT,
) -> Option<Vec<PDH_FMT_COUNTERVALUE_ITEM_W>> {
    let mut buffer_size: u32 = 0;
    let mut item_count: u32 = 0;
    let _ = PdhGetFormattedCounterArrayW(counter, fmt, &mut buffer_size, &mut item_count, None);
    if buffer_size == 0 || item_count == 0 {
        return None;
    }
    let mut buf: Vec<u8> = vec![0; buffer_size as usize];
    let res = PdhGetFormattedCounterArrayW(
        counter,
        fmt,
        &mut buffer_size,
        &mut item_count,
        Some(buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
    );
    if res != 0 {
        return None;
    }
    let items_ptr = buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W;
    let items = std::slice::from_raw_parts(items_ptr, item_count as usize);
    Some(items.to_vec())
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.close_pdh();
    }
}
