// PresentMon 스타일 FPS 측정. ferrisetw로 Microsoft-Windows-DXGI provider의
// PresentStart 이벤트를 user-mode ETW 세션에서 구독하고, 1초 sliding window로 카운팅.
//
// 콜백은 ferrisetw가 spawn한 별도 worker thread에서 실행된다.
// FpsSession은 main 스레드에서 `start()`로 생성, Drop 시 ETW 세션 `stop`.
// 현재 FPS는 `Arc<AtomicU32>`로 공유, main 스레드는 `current_fps()`로 lock 없이 읽는다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::UserTrace;
use ferrisetw::EventRecord;

// Microsoft-Windows-DXGI provider GUID
const DXGI_PROVIDER_GUID: &str = "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9";

// DXGI PresentStart 이벤트 ID. 실측 검증에서 다르면 fallback 노트 참조.
const PRESENT_START_EVENT_ID: u16 = 42;
const PRESENT_EVENT_TTL: Duration = Duration::from_secs(1);

const SESSION_NAME: &str = "bdo-optimizer-fps";
static START_CLAIM_GENERATION: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StartClaim(u64);

fn advance_start_claim(generation: &AtomicU64) -> u64 {
    generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn start_claim_is_current(generation: &AtomicU64, claim: u64) -> bool {
    generation.load(Ordering::Acquire) == claim
}

pub(crate) fn claim_start() -> StartClaim {
    StartClaim(advance_start_claim(&START_CLAIM_GENERATION))
}

pub(crate) fn invalidate_start_claims() {
    advance_start_claim(&START_CLAIM_GENERATION);
}

struct SessionOwner<T> {
    generation: u64,
    session: Option<T>,
}

impl<T> Default for SessionOwner<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            session: None,
        }
    }
}

fn replace_owned_session<T, E>(
    owner: &Mutex<SessionOwner<T>>,
    claim_is_current: impl FnOnce() -> bool,
    stop: impl FnOnce(T),
    start: impl FnOnce() -> Result<T, E>,
) -> Result<Option<u64>, E> {
    let mut owner = owner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !claim_is_current() {
        return Ok(None);
    }
    owner.generation = owner.generation.wrapping_add(1);
    let generation = owner.generation;
    if let Some(previous) = owner.session.take() {
        stop(previous);
    }
    owner.session = Some(start()?);
    Ok(Some(generation))
}

fn stop_owned_session<T>(owner: &Mutex<SessionOwner<T>>, generation: u64, stop: impl FnOnce(T)) {
    let mut owner = owner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if owner.generation != generation {
        return;
    }
    owner.generation = owner.generation.wrapping_add(1);
    if let Some(session) = owner.session.take() {
        stop(session);
    }
}

fn session_owner() -> &'static Mutex<SessionOwner<UserTrace>> {
    static OWNER: OnceLock<Mutex<SessionOwner<UserTrace>>> = OnceLock::new();
    OWNER.get_or_init(|| Mutex::new(SessionOwner::default()))
}

// M66b: thiserror enum. 호출처 메시지(`ETW 세션 시작 실패: {:?}`)와 동일하게 유지.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ETW 세션 시작 실패: {0}")]
    EtwStart(String),
    #[error("ETW 세션 시작 요청이 최신 요청으로 대체되었습니다.")]
    StaleStart,
}

// 이전 앱 인스턴스가 비정상 종료(강제 종료/패닉/디버거 stop 등)되면 같은 이름의
// ETW 세션이 시스템에 남아 다음 실행 시 `AlreadyExist`로 시작 실패한다.
// 시작 전 무조건 stop 시도(없으면 logman이 비제로 종료, 무시).
fn stop_stale_session() {
    let _ = ferrisetw::trace::stop_trace_by_name(SESSION_NAME);
}

pub struct FpsSession {
    timestamps: Arc<Mutex<VecDeque<Instant>>>,
    total_events: Arc<AtomicU64>,
    present_events: Arc<AtomicU64>,
    generation: u64,
}

struct CallbackState {
    timestamps: Arc<Mutex<VecDeque<Instant>>>,
    total_events: Arc<AtomicU64>,
    present_events: Arc<AtomicU64>,
}

impl FpsSession {
    pub(crate) fn start(target_pid: u32, claim: StartClaim) -> Result<Self, Error> {
        let timestamps = Arc::new(Mutex::new(VecDeque::with_capacity(256)));
        let total_events = Arc::new(AtomicU64::new(0));
        let present_events = Arc::new(AtomicU64::new(0));
        let state = Arc::new(CallbackState {
            timestamps: Arc::clone(&timestamps),
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
                if !is_target_present_event(record.event_id(), record.process_id(), target_pid) {
                    return;
                }
                state_for_cb.present_events.fetch_add(1, Ordering::Relaxed);
                let now = Instant::now();
                if let Ok(mut ts) = state_for_cb.timestamps.lock() {
                    ts.push_back(now);
                    prune_and_count(&mut ts, now);
                }
            })
            .build();

        let generation = replace_owned_session(
            session_owner(),
            || start_claim_is_current(&START_CLAIM_GENERATION, claim.0),
            drop,
            || {
                stop_stale_session();
                UserTrace::new()
                    .named(SESSION_NAME.to_string())
                    .enable(provider)
                    .start_and_process()
                    .map_err(|e| Error::EtwStart(format!("{:?}", e)))
            },
        )?
        .ok_or(Error::StaleStart)?;

        Ok(Self {
            timestamps,
            total_events,
            present_events,
            generation,
        })
    }

    pub fn current_fps(&self) -> u32 {
        self.timestamps
            .lock()
            .map(|mut timestamps| prune_and_count(&mut timestamps, Instant::now()))
            .unwrap_or(0)
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
        stop_owned_session(session_owner(), self.generation, drop);
    }
}

fn is_target_present_event(event_id: u16, event_pid: u32, target_pid: u32) -> bool {
    event_id == PRESENT_START_EVENT_ID && event_pid == target_pid
}

fn prune_and_count(timestamps: &mut VecDeque<Instant>, now: Instant) -> u32 {
    while timestamps
        .front()
        .is_some_and(|timestamp| now.saturating_duration_since(*timestamp) >= PRESENT_EVENT_TTL)
    {
        timestamps.pop_front();
    }
    timestamps.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn present_event_must_belong_to_the_target_game_pid() {
        assert!(is_target_present_event(42, 700, 700));
        assert!(!is_target_present_event(42, 701, 700));
        assert!(!is_target_present_event(41, 700, 700));
    }

    #[test]
    fn fps_decays_after_present_event_ttl() {
        let now = Instant::now();
        let mut timestamps = VecDeque::from([
            now - Duration::from_millis(1001),
            now - Duration::from_millis(999),
            now - Duration::from_millis(400),
        ]);
        assert_eq!(prune_and_count(&mut timestamps, now), 2);
        assert_eq!(
            prune_and_count(&mut timestamps, now + Duration::from_secs(1)),
            0
        );
    }

    #[test]
    fn stale_session_drop_cannot_stop_the_latest_named_trace() {
        let owner = Mutex::new(SessionOwner::<&'static str>::default());
        let latest_claim = AtomicU64::new(0);
        let stopped = Mutex::new(Vec::new());

        let first_claim = advance_start_claim(&latest_claim);
        let first = replace_owned_session(
            &owner,
            || start_claim_is_current(&latest_claim, first_claim),
            |trace| {
                assert!(matches!(
                    owner.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                stopped
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(trace);
            },
            || {
                assert!(matches!(
                    owner.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                Ok::<_, ()>("first")
            },
        )
        .unwrap()
        .unwrap();
        let second_claim = advance_start_claim(&latest_claim);
        let second = replace_owned_session(
            &owner,
            || start_claim_is_current(&latest_claim, second_claim),
            |trace| {
                assert!(matches!(
                    owner.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                stopped
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(trace);
            },
            || {
                assert!(matches!(
                    owner.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                Ok::<_, ()>("second")
            },
        )
        .unwrap()
        .unwrap();

        stop_owned_session(&owner, first, |trace| {
            stopped
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(trace);
        });
        assert_eq!(
            owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .session,
            Some("second")
        );

        stop_owned_session(&owner, second, |trace| {
            stopped
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(trace);
        });
        assert_eq!(
            *stopped
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["first", "second"]
        );
    }

    #[test]
    fn late_stale_start_cannot_replace_the_latest_claimed_session() {
        let owner = Arc::new(Mutex::new(SessionOwner::<&'static str>::default()));
        let latest_claim = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(Mutex::new(Vec::new()));
        let stale_ready = Arc::new(Barrier::new(2));
        let release_stale = Arc::new(Barrier::new(2));

        let stale_claim = advance_start_claim(&latest_claim);
        let stale_owner = Arc::clone(&owner);
        let stale_latest_claim = Arc::clone(&latest_claim);
        let stale_stopped = Arc::clone(&stopped);
        let stale_ready_worker = Arc::clone(&stale_ready);
        let release_stale_worker = Arc::clone(&release_stale);
        let stale_worker = thread::spawn(move || {
            stale_ready_worker.wait();
            release_stale_worker.wait();
            replace_owned_session(
                &stale_owner,
                || start_claim_is_current(&stale_latest_claim, stale_claim),
                |trace| {
                    stale_stopped
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(trace);
                },
                || Ok::<_, ()>("stale"),
            )
        });

        stale_ready.wait();
        advance_start_claim(&latest_claim); // stop/invalidate
        let current_claim = advance_start_claim(&latest_claim);
        let current_generation = replace_owned_session(
            &owner,
            || start_claim_is_current(&latest_claim, current_claim),
            |trace| {
                stopped
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(trace);
            },
            || Ok::<_, ()>("latest"),
        )
        .unwrap()
        .expect("latest claim must start");

        release_stale.wait();
        assert_eq!(stale_worker.join().unwrap().unwrap(), None);
        assert_eq!(
            owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .session,
            Some("latest")
        );
        assert!(stopped
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());

        stop_owned_session(&owner, current_generation, drop);
    }

    #[test]
    fn invalidated_start_claim_cannot_mutate_an_empty_owner() {
        let owner = Mutex::new(SessionOwner::<&'static str>::default());
        let latest_claim = AtomicU64::new(0);
        let stop_calls = std::cell::Cell::new(0);
        let start_calls = std::cell::Cell::new(0);
        let stale_claim = advance_start_claim(&latest_claim);
        advance_start_claim(&latest_claim);

        let result = replace_owned_session(
            &owner,
            || start_claim_is_current(&latest_claim, stale_claim),
            |_| stop_calls.set(stop_calls.get() + 1),
            || {
                start_calls.set(start_calls.get() + 1);
                Ok::<_, ()>("stale")
            },
        )
        .unwrap();

        assert_eq!(result, None);
        assert_eq!(stop_calls.get(), 0);
        assert_eq!(start_calls.get(), 0);
        let owner = owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(owner.generation, 0);
        assert_eq!(owner.session, None);
    }
}
