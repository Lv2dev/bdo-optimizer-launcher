// PresentMon 스타일 FPS 측정. ferrisetw로 Microsoft-Windows-DXGI provider의
// PresentStart 이벤트를 user-mode ETW 세션에서 구독하고, 1초 sliding window로 카운팅.
//
// 콜백은 ferrisetw가 spawn한 별도 worker thread에서 실행된다.
// FpsSession은 main 스레드에서 `start(pid)`로 생성, Drop 시 ETW 세션 `stop`.
// 현재 FPS는 `Arc<AtomicU32>`로 공유, main 스레드는 `current_fps()`로 lock 없이 읽는다.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::UserTrace;
use ferrisetw::EventRecord;

// Microsoft-Windows-DXGI provider GUID
const DXGI_PROVIDER_GUID: &str = "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9";

// DXGI PresentStart 이벤트 ID. 실측 검증에서 다르면 fallback 노트 참조.
const PRESENT_START_EVENT_ID: u16 = 42;

const SESSION_NAME: &str = "bdo-optimizer-fps";

// M66b: thiserror enum. 호출처 메시지(`ETW 세션 시작 실패: {:?}`)와 동일하게 유지.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ETW 세션 시작 실패: {0}")]
    EtwStart(String),
}

// 이전 앱 인스턴스가 비정상 종료(강제 종료/패닉/디버거 stop 등)되면 같은 이름의
// ETW 세션이 시스템에 남아 다음 실행 시 `AlreadyExist`로 시작 실패한다.
// 시작 전 무조건 stop 시도(없으면 logman이 비제로 종료, 무시).
fn stop_stale_session() {
    let _ = super::system_command("logman.exe")
        .args(["stop", SESSION_NAME, "-ets"])
        .output();
}

pub struct FpsSession {
    current_fps: Arc<AtomicU32>,
    total_events: Arc<AtomicU64>,
    present_events: Arc<AtomicU64>,
    trace: Option<UserTrace>,
}

struct CallbackState {
    pid: u32,
    timestamps: Mutex<Vec<Instant>>,
    current_fps: Arc<AtomicU32>,
    total_events: Arc<AtomicU64>,
    present_events: Arc<AtomicU64>,
}

impl FpsSession {
    pub fn start(pid: u32) -> Result<Self, Error> {
        stop_stale_session();

        let current_fps = Arc::new(AtomicU32::new(0));
        let total_events = Arc::new(AtomicU64::new(0));
        let present_events = Arc::new(AtomicU64::new(0));
        let state = Arc::new(CallbackState {
            pid,
            timestamps: Mutex::new(Vec::with_capacity(256)),
            current_fps: Arc::clone(&current_fps),
            total_events: Arc::clone(&total_events),
            present_events: Arc::clone(&present_events),
        });

        let state_for_cb = Arc::clone(&state);
        // PresentMon 패턴: DXGI provider는 keyword/level 명시 없이는 Present 이벤트를 emit 안 함.
        // keyword 0xFFFF로 광범위 활성, level VERBOSE(5). PID 필터는 process_id가 dwm로 들어오는 경우가 있어 제거.
        let provider = Provider::by_guid(DXGI_PROVIDER_GUID)
            .any(0xFFFF)
            .level(5)
            .add_callback(move |record: &EventRecord, _schema: &SchemaLocator| {
                state_for_cb.total_events.fetch_add(1, Ordering::Relaxed);
                if record.event_id() != PRESENT_START_EVENT_ID {
                    return;
                }
                state_for_cb.present_events.fetch_add(1, Ordering::Relaxed);
                let _ = state_for_cb.pid;
                let now = Instant::now();
                if let Ok(mut ts) = state_for_cb.timestamps.lock() {
                    ts.push(now);
                    let cutoff = now - std::time::Duration::from_secs(1);
                    ts.retain(|t| *t >= cutoff);
                    state_for_cb
                        .current_fps
                        .store(ts.len() as u32, Ordering::Relaxed);
                }
            })
            .build();

        let trace = UserTrace::new()
            .named(SESSION_NAME.to_string())
            .enable(provider)
            .start_and_process()
            .map_err(|e| Error::EtwStart(format!("{:?}", e)))?;

        Ok(Self {
            current_fps,
            total_events,
            present_events,
            trace: Some(trace),
        })
    }

    pub fn current_fps(&self) -> u32 {
        self.current_fps.load(Ordering::Relaxed)
    }

    pub fn total_events(&self) -> u64 {
        self.total_events.load(Ordering::Relaxed)
    }

    pub fn present_events(&self) -> u64 {
        self.present_events.load(Ordering::Relaxed)
    }
}

impl Drop for FpsSession {
    fn drop(&mut self) {
        if let Some(trace) = self.trace.take() {
            let _ = trace.stop();
        }
    }
}
