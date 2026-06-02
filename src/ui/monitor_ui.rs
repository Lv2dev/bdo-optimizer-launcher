use crate::backend::monitor::MonitorSample;
use crate::backend::process;
use crate::ui::{AppState, MonitorState, SparkBuffers};
use crate::AppWindow;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

const SPARK_LEN: usize = 60;
// viewbox 좌표계: 가로로 긴 직사각형(10:1)으로 두어 박스 aspect와 비슷하게 만든다.
// 정사각형 viewbox(100×100)는 Slint Path의 aspect 보존 때문에 박스 가운데에만 fit됨.
const VIEW_W: f32 = 1000.0;
const VIEW_H: f32 = 100.0;

pub fn fmt_pct(v: Option<f64>) -> String {
    match v {
        Some(p) => format!("{:.1}%", p),
        None => "--".to_string(),
    }
}

#[allow(dead_code)]
pub fn fmt_mb(v: Option<u64>) -> String {
    match v {
        Some(n) => format!("{} MB", n),
        None => "--".to_string(),
    }
}

// "1.2 / 16.0 GB" 형식. total이 0이면 단일 값만.
pub fn fmt_gb_of_total(used_mb: Option<u64>, total_mb: u64) -> String {
    match used_mb {
        Some(u) => {
            let u_gb = u as f64 / 1024.0;
            if total_mb > 0 {
                let t_gb = total_mb as f64 / 1024.0;
                format!("{:.1} / {:.1} GB", u_gb, t_gb)
            } else {
                format!("{:.1} GB", u_gb)
            }
        }
        None => "--".to_string(),
    }
}

// "1234 / 8192 MB" 형식. total이 0이면 단일 값만.
pub fn fmt_mb_of_total(used_mb: Option<u64>, total_mb: u64) -> String {
    match used_mb {
        Some(u) => {
            if total_mb > 0 {
                format!("{} / {} MB", u, total_mb)
            } else {
                format!("{} MB", u)
            }
        }
        None => "--".to_string(),
    }
}

pub fn fmt_kbs(v: Option<u64>) -> String {
    match v {
        Some(n) => format!("{} KB/s", n),
        None => "--".to_string(),
    }
}

// VecDeque를 60칸 sliding window로 유지하면서 새 값 push.
fn push_spark(buf: &Rc<RefCell<VecDeque<f32>>>, v: f32) {
    let mut b = buf.borrow_mut();
    if b.len() >= SPARK_LEN {
        b.pop_front();
    }
    b.push_back(v);
}

// Catmull-Rom → cubic Bezier 변환. 결과는 (stroke 전용 open path, fill 전용 closed path).
// open path는 polyline만, closed path는 마지막 점→우하단→좌하단→Z로 fill 영역을 정의.
// max는 cap(최대 허용값). 실제 정규화는 buffer 내 관측 최대 + 15% margin과 cap의 작은 쪽으로 자동 스케일링.
// → 그래프가 카드 전체를 거의 채우게 됨(작업관리자 동적 스케일링 패턴).
fn spark_paths(buf: &Rc<RefCell<VecDeque<f32>>>, cap: f32) -> (String, String) {
    let b = buf.borrow();
    spark_paths_from_deque(&b, cap, true)
}

// dynamic_scale = true면 buffer 내 관측 max * 1.15로 자동 스케일링(cap 상한).
// false면 cap을 그대로 max로 사용 — 코어 간 비교가 필요한 경우(코어별 mini 차트) 쓴다.
fn spark_paths_from_deque(b: &VecDeque<f32>, cap: f32, dynamic_scale: bool) -> (String, String) {
    let n = b.len();
    if n < 2 {
        return (String::new(), String::new());
    }
    let max = if dynamic_scale {
        let observed = b.iter().fold(0.0f32, |a, v| a.max(*v));
        let dynamic = (observed * 1.15).max(1.0);
        if cap > 0.0 {
            dynamic.min(cap)
        } else {
            dynamic
        }
    } else {
        if cap > 0.0 {
            cap
        } else {
            1.0
        }
    };
    let max = if max <= 0.0 { 1.0 } else { max };
    // 우측 정렬: 새 데이터는 x=VIEW_W(오른쪽 끝)에서 시작해 왼쪽으로 흐름.
    // buffer가 일부만 차 있어도 마지막 점은 항상 x=VIEW_W.
    let offset = SPARK_LEN.saturating_sub(n);
    let points: Vec<(f32, f32)> = b
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = (offset + i) as f32 / (SPARK_LEN - 1) as f32 * VIEW_W;
            let y = (1.0 - (v / max).clamp(0.0, 1.0)) * VIEW_H;
            (x, y)
        })
        .collect();

    let mut stroke = String::with_capacity(n * 32);
    stroke.push_str(&format!("M {:.2} {:.2}", points[0].0, points[0].1));

    if n == 2 {
        stroke.push_str(&format!(" L {:.2} {:.2}", points[1].0, points[1].1));
    } else {
        let tau: f32 = 0.35;
        for i in 0..n - 1 {
            let p0 = if i == 0 { points[0] } else { points[i - 1] };
            let p1 = points[i];
            let p2 = points[i + 1];
            let p3 = if i + 2 < n {
                points[i + 2]
            } else {
                points[i + 1]
            };
            let cp1_x = p1.0 + (p2.0 - p0.0) * tau / 3.0;
            let cp1_y = p1.1 + (p2.1 - p0.1) * tau / 3.0;
            let cp2_x = p2.0 - (p3.0 - p1.0) * tau / 3.0;
            let cp2_y = p2.1 - (p3.1 - p1.1) * tau / 3.0;
            stroke.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                cp1_x, cp1_y, cp2_x, cp2_y, p2.0, p2.1
            ));
        }
    }

    // fill path: stroke를 그대로 따라가다 우하단 → 좌하단 → Z로 닫음.
    let mut fill = stroke.clone();
    let last_x = points[n - 1].0;
    let first_x = points[0].0;
    fill.push_str(&format!(
        " L {:.2} {:.0} L {:.2} {:.0} Z",
        last_x, VIEW_H, first_x, VIEW_H
    ));

    (stroke, fill)
}

// M84 (MJ-4): FPS 표시 분류 결과. compute_fps_display의 pure 반환.
#[derive(Clone, PartialEq, Debug)]
struct FpsDisplay {
    text: String,
    pct: f32,
}

// M84: FPS 진단 텍스트 + percent 계산 분리(pure fn, 단위 테스트 가능).
// 호출처는 `state.game.fps_session.borrow()`로 4 값(current_fps/present_events/total_events/alive)을
// 추출한 뒤 본 함수에 전달.
fn compute_fps_display(
    current_fps: u32,
    present_events: u64,
    total_events: u64,
    alive: bool,
) -> FpsDisplay {
    let text = if !alive {
        "세션 미시작".to_string()
    } else if current_fps > 0 {
        format!("{} FPS", current_fps)
    } else if present_events > 0 {
        "측정 중...".to_string()
    } else if total_events > 0 {
        format!("Present 미수신 ({} ev)", total_events)
    } else {
        "ETW 이벤트 없음".to_string()
    };
    let pct = ((current_fps as f32) / 240.0 * 100.0).clamp(0.0, 100.0);
    FpsDisplay { text, pct }
}

// M84: total_mb > 0이면 그대로, 아니면 default fallback. mem/vram cap 계산에 4회 반복되던 패턴 통합.
fn cap_or_default(total: u64, default: f32) -> f32 {
    if total > 0 {
        total as f32
    } else {
        default
    }
}

// M84: CPU/MEM/GPU/VRAM/Disk/PID 메인 metric 11개 Slint property set 묶음.
fn update_main_metrics(
    app: &AppWindow,
    pid: u32,
    sample: &MonitorSample,
    total_ram_mb: u64,
    total_vram_mb: u64,
    mem_cap: f32,
    vram_cap: f32,
) {
    app.set_mon_pid(format!("{}", pid).into());
    app.set_mon_cpu(fmt_pct(sample.cpu_pct).into());
    app.set_mon_mem_mb(fmt_gb_of_total(sample.mem_mb, total_ram_mb).into());
    app.set_mon_gpu(fmt_pct(sample.gpu_pct).into());
    app.set_mon_gpu_pct(sample.gpu_pct.unwrap_or(0.0) as f32);
    app.set_mon_cpu_pct(sample.cpu_pct.unwrap_or(0.0) as f32);
    app.set_mon_mem_pct(((sample.mem_mb.unwrap_or(0) as f32) / mem_cap * 100.0).clamp(0.0, 100.0));
    app.set_mon_vram_mb(fmt_mb_of_total(sample.vram_mb, total_vram_mb).into());
    app.set_mon_vram_pct(
        ((sample.vram_mb.unwrap_or(0) as f32) / vram_cap * 100.0).clamp(0.0, 100.0),
    );
    app.set_mon_disk_r(fmt_kbs(sample.disk_read_kbs).into());
    app.set_mon_disk_w(fmt_kbs(sample.disk_write_kbs).into());
}

// M84: 5 채널 sparkline buffer push + Catmull-Rom path 생성 + 10 Slint property set 묶음.
fn update_main_sparklines(
    app: &AppWindow,
    spark: &SparkBuffers,
    sample: &MonitorSample,
    fps_value: u32,
    mem_cap: f32,
    vram_cap: f32,
) {
    // CPU: 0~100, MEM: WorkingSet 기준 ~16GB cap, GPU: 0~100, FPS: ~240 cap.
    push_spark(&spark.cpu, sample.cpu_pct.unwrap_or(0.0) as f32);
    push_spark(&spark.mem, sample.mem_mb.unwrap_or(0) as f32);
    push_spark(&spark.gpu, sample.gpu_pct.unwrap_or(0.0) as f32);
    push_spark(&spark.vram, sample.vram_mb.unwrap_or(0) as f32);
    push_spark(&spark.fps, fps_value as f32);
    let (cpu_s, cpu_f) = spark_paths(&spark.cpu, 100.0);
    let (mem_s, mem_f) = spark_paths(&spark.mem, mem_cap);
    let (gpu_s, gpu_f) = spark_paths(&spark.gpu, 100.0);
    let (vram_s, vram_f) = spark_paths(&spark.vram, vram_cap);
    let (fps_s, fps_f) = spark_paths(&spark.fps, 240.0);
    app.set_mon_cpu_spark(cpu_s.into());
    app.set_mon_cpu_spark_fill(cpu_f.into());
    app.set_mon_mem_spark(mem_s.into());
    app.set_mon_mem_spark_fill(mem_f.into());
    app.set_mon_gpu_spark(gpu_s.into());
    app.set_mon_gpu_spark_fill(gpu_f.into());
    app.set_mon_vram_spark(vram_s.into());
    app.set_mon_vram_spark_fill(vram_f.into());
    app.set_mon_fps_spark(fps_s.into());
    app.set_mon_fps_spark_fill(fps_f.into());
}

// M84: 코어별 sparkline buffer 동기화 + 4 VecModel resize + per-row in-place 갱신 묶음.
// M83 substate 효과로 MonitorState만 받아 도메인 명확.
fn update_core_metrics(monitor: &MonitorState, sample: &MonitorSample) {
    let core_count = sample.core_usages.len();
    let mask = sample.affinity_mask.unwrap_or(usize::MAX);

    // 코어별 sparkline buffer 동기화 (코어 수 변동은 사실상 없으나 안전망).
    {
        let mut cores_buf = monitor.spark.cores.borrow_mut();
        if cores_buf.len() != core_count {
            cores_buf.clear();
            for _ in 0..core_count {
                cores_buf.push(VecDeque::with_capacity(SPARK_LEN));
            }
        }
        for (i, v) in sample.core_usages.iter().enumerate() {
            if i < cores_buf.len() {
                let q = &mut cores_buf[i];
                if q.len() >= SPARK_LEN {
                    q.pop_front();
                }
                q.push_back(*v as f32);
            }
        }
    }

    // VecModel row_count 동기화 (M81 in-place 갱신 패턴).
    resize_vec_model(&monitor.mon_cores_vec, core_count, 0.0_f32);
    resize_vec_model(&monitor.mon_cores_active_vec, core_count, false);
    resize_vec_model(
        &monitor.mon_core_sparks_vec,
        core_count,
        SharedString::default(),
    );
    resize_vec_model(
        &monitor.mon_core_sparks_fill_vec,
        core_count,
        SharedString::default(),
    );

    // 각 row in-place 갱신.
    let cores_buf = monitor.spark.cores.borrow();
    for i in 0..core_count {
        let usage = sample.core_usages[i] as f32;
        monitor.mon_cores_vec.set_row_data(i, usage);
        monitor
            .mon_cores_active_vec
            .set_row_data(i, (mask >> i) & 1 == 1);
        // 코어 간 비교가 핵심이므로 dynamic_scale=false, max=100 고정.
        let (stroke, fill) = spark_paths_from_deque(&cores_buf[i], 100.0, false);
        monitor.mon_core_sparks_vec.set_row_data(i, stroke.into());
        monitor
            .mon_core_sparks_fill_vec
            .set_row_data(i, fill.into());
    }
}

pub fn apply_initial(app: &AppWindow) {
    let info = crate::backend::system_info::fetch_system_info();
    app.set_sys_cpu_name(info.cpu_name.into());
    let gpu_text = if info.gpu_names.is_empty() {
        "Unknown GPU".to_string()
    } else {
        info.gpu_names.join(" / ")
    };
    app.set_sys_gpu_name(gpu_text.into());
}

pub fn register(app: &AppWindow) {
    app.on_toggle_mon_cores_view({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let cur = app.get_mon_cores_view();
                app.set_mon_cores_view(!cur);
            }
        }
    });

    app.on_toggle_mon_core_detail({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let cur = app.get_mon_core_detail_chart();
                app.set_mon_core_detail_chart(!cur);
            }
        }
    });

    app.on_toggle_mon_main_view({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                let cur = app.get_mon_main_chart_view();
                app.set_mon_main_chart_view(!cur);
            }
        }
    });
}

pub fn start_monitor_timer(app: &AppWindow, state: &AppState) -> slint::Timer {
    // M81 (Mj1): 4 VecModel을 한 번만 Slint property에 set.
    // 이후 매 tick은 row_count 동기화 + set_row_data로 in-place 갱신해
    // ModelRc/VecModel/Vec 재생성 alloc을 제거한다.
    app.set_mon_cores(ModelRc::from(state.monitor.mon_cores_vec.clone()));
    app.set_mon_cores_active(ModelRc::from(state.monitor.mon_cores_active_vec.clone()));
    app.set_mon_core_sparks(ModelRc::from(state.monitor.mon_core_sparks_vec.clone()));
    app.set_mon_core_sparks_fill(ModelRc::from(
        state.monitor.mon_core_sparks_fill_vec.clone(),
    ));

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(1),
        {
            let app = app.as_weak();
            let state = state.clone();
            move || {
                let Some(app) = app.upgrade() else {
                    return;
                };
                if !app.get_game_running() || app.get_active_tab() != 2 {
                    return;
                }
                let Some(pid) = process::find_process_id("BlackDesert64.exe") else {
                    return;
                };

                let (total_ram_mb, total_vram_mb) = {
                    let m = state.monitor.game_monitor.borrow();
                    (m.total_ram_mb, m.total_vram_mb)
                };
                let sample = state.monitor.game_monitor.borrow_mut().sample(pid);
                let mem_cap = cap_or_default(total_ram_mb, 16384.0);
                let vram_cap = cap_or_default(total_vram_mb, 8192.0);

                update_main_metrics(
                    &app,
                    pid,
                    &sample,
                    total_ram_mb,
                    total_vram_mb,
                    mem_cap,
                    vram_cap,
                );

                // M84: FPS 진단 — fps_session에서 추출한 raw value를 sparkline에도 쓰므로 명시 분리.
                let (fps_value, fps_display) = {
                    let session = state.game.fps_session.borrow();
                    match session.as_ref() {
                        Some(s) => {
                            let v = s.current_fps();
                            (
                                v,
                                compute_fps_display(v, s.present_events(), s.total_events(), true),
                            )
                        }
                        None => (0, compute_fps_display(0, 0, 0, false)),
                    }
                };
                app.set_mon_fps(fps_display.text.into());
                app.set_mon_fps_pct(fps_display.pct);

                update_main_sparklines(
                    &app,
                    &state.monitor.spark,
                    &sample,
                    fps_value,
                    mem_cap,
                    vram_cap,
                );
                update_core_metrics(&state.monitor, &sample);
            }
        },
    );
    timer
}

// VecModel의 row_count를 target_len에 맞춰 push/remove로 in-place 동기화.
// row가 부족하면 default 값으로 push, 넘치면 끝에서 remove.
fn resize_vec_model<T: Clone + 'static>(model: &VecModel<T>, target_len: usize, default: T) {
    while model.row_count() > target_len {
        model.remove(model.row_count() - 1);
    }
    while model.row_count() < target_len {
        model.push(default.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_fps_display_session_not_alive_shows_no_session() {
        let r = compute_fps_display(0, 0, 0, false);
        assert_eq!(r.text, "세션 미시작");
        assert_eq!(r.pct, 0.0);
    }

    #[test]
    fn compute_fps_display_session_alive_no_events() {
        let r = compute_fps_display(0, 0, 0, true);
        assert_eq!(r.text, "ETW 이벤트 없음");
        assert_eq!(r.pct, 0.0);
    }

    #[test]
    fn compute_fps_display_total_events_but_no_present() {
        let r = compute_fps_display(0, 0, 42, true);
        assert_eq!(r.text, "Present 미수신 (42 ev)");
        assert_eq!(r.pct, 0.0);
    }

    #[test]
    fn compute_fps_display_present_events_no_fps_yet() {
        let r = compute_fps_display(0, 5, 5, true);
        assert_eq!(r.text, "측정 중...");
        assert_eq!(r.pct, 0.0);
    }

    #[test]
    fn compute_fps_display_normal_fps() {
        let r = compute_fps_display(60, 60, 60, true);
        assert_eq!(r.text, "60 FPS");
        assert!((r.pct - 25.0).abs() < 0.01);
    }

    #[test]
    fn compute_fps_display_caps_pct_at_100() {
        let r = compute_fps_display(300, 300, 300, true);
        assert_eq!(r.text, "300 FPS");
        assert_eq!(r.pct, 100.0);
    }

    #[test]
    fn cap_or_default_uses_total_when_positive() {
        assert_eq!(cap_or_default(32768, 16384.0), 32768.0);
        assert_eq!(cap_or_default(1, 16384.0), 1.0);
    }

    #[test]
    fn cap_or_default_falls_back_to_default_when_zero() {
        assert_eq!(cap_or_default(0, 16384.0), 16384.0);
        assert_eq!(cap_or_default(0, 8192.0), 8192.0);
    }
}
