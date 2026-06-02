use crate::backend::{fps, monitor, schedule, tray};
use crate::ScheduleRuleUi;
use slint::{SharedString, VecModel};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub mod calendar;
pub mod control;
pub mod monitor_ui;
pub mod schedule_ui;
pub mod settings;
pub mod shutdown;
pub mod tray_ui;
pub mod updates;

// Sparkline 60칸 sliding window 버퍼 모음. CPU/MEM/GPU/VRAM/FPS 5채널.
#[derive(Clone, Default)]
pub struct SparkBuffers {
    pub cpu: Rc<RefCell<VecDeque<f32>>>,
    pub mem: Rc<RefCell<VecDeque<f32>>>,
    pub gpu: Rc<RefCell<VecDeque<f32>>>,
    pub vram: Rc<RefCell<VecDeque<f32>>>,
    pub fps: Rc<RefCell<VecDeque<f32>>>,
    // 시스템 코어별 60칸 sliding window. 코어 수만큼 동적 확장.
    pub cores: Rc<RefCell<Vec<VecDeque<f32>>>>,
}

// M83 (MJ-3): AppState 13 필드를 4 도메인 substate로 분리해 책임 경계 명확화.
// 각 substate는 Rc/SparkBuffers/VecModel만 보유하므로 #[derive(Clone)] 비용 0.

#[derive(Clone)]
pub struct ScheduleState {
    pub rules: Rc<RefCell<Vec<schedule::ScheduleRule>>>,
    pub rules_model: Rc<VecModel<ScheduleRuleUi>>,
}

#[derive(Clone)]
pub struct GameState {
    pub last_game_state: Rc<RefCell<bool>>,
    pub fps_session: Rc<RefCell<Option<fps::FpsSession>>>,
    // 게임 창 가시성 직전 상태(true=보임). status_timer가 toggle 감지에 사용.
    pub prev_game_visible: Rc<RefCell<bool>>,
}

#[derive(Clone)]
pub struct ModeState {
    // 게임가드 우회를 위한 단기 재적용 single-shot 타이머 5개. 새 모드 적용 시 교체되어 자연 취소.
    pub reapply_timers: Rc<RefCell<Vec<slint::Timer>>>,
    // 자동 저전력 모드 진입 직전의 사용자 모드. 게임 창이 다시 보이면 이 모드로 복원.
    pub last_user_mode: Rc<RefCell<Option<schedule::OptimizeMode>>>,
    // current-mode 갱신 시 트레이 tooltip/menu indicator도 함께 동기화하기 위한 핸들.
    pub tray_handle: Rc<RefCell<Option<Rc<tray::TrayHandle>>>>,
}

#[derive(Clone)]
pub struct MonitorState {
    pub game_monitor: Rc<RefCell<monitor::Monitor>>,
    pub spark: SparkBuffers,
    // M81 (Mj1): 매 monitor tick ModelRc 재생성 회피.
    // 첫 tick에 한 번만 app에 set하고, 이후 row_count 동기화 + set_row_data로 in-place 갱신.
    pub mon_cores_vec: Rc<VecModel<f32>>,
    pub mon_cores_active_vec: Rc<VecModel<bool>>,
    pub mon_core_sparks_vec: Rc<VecModel<SharedString>>,
    pub mon_core_sparks_fill_vec: Rc<VecModel<SharedString>>,
}

#[derive(Clone)]
pub struct AppState {
    pub schedule: ScheduleState,
    pub game: GameState,
    pub mode: ModeState,
    pub monitor: MonitorState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            schedule: ScheduleState {
                rules: Rc::new(RefCell::new(schedule::load_rules())),
                rules_model: Rc::new(VecModel::default()),
            },
            game: GameState {
                last_game_state: Rc::new(RefCell::new(false)),
                fps_session: Rc::new(RefCell::new(None)),
                prev_game_visible: Rc::new(RefCell::new(true)),
            },
            mode: ModeState {
                reapply_timers: Rc::new(RefCell::new(Vec::new())),
                last_user_mode: Rc::new(RefCell::new(None)),
                tray_handle: Rc::new(RefCell::new(None)),
            },
            monitor: MonitorState {
                game_monitor: Rc::new(RefCell::new(monitor::Monitor::new())),
                spark: SparkBuffers::default(),
                mon_cores_vec: Rc::new(VecModel::default()),
                mon_cores_active_vec: Rc::new(VecModel::default()),
                mon_core_sparks_vec: Rc::new(VecModel::default()),
                mon_core_sparks_fill_vec: Rc::new(VecModel::default()),
            },
        }
    }
}

// 도메인 모듈 간 공통: 상태바 메시지 갱신.
pub fn push_status(app: &crate::AppWindow, msg: impl Into<slint::SharedString>) {
    let prev = app.get_status_text();
    app.set_prev_status_text(prev);
    app.set_status_text(msg.into());
}
