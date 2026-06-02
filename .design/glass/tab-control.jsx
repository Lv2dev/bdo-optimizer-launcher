// tab-control.jsx — 제어 (Control)
function ControlTab({ state, set, toast }) {
  const { I } = window;
  const modes = {
    high:   { label: "고성능", icon: I.gauge, badge: "성능 우선" },
    normal: { label: "일반",   icon: I.bolt,  badge: "균형" },
    low:    { label: "저전력", icon: I.leaf,  badge: "절전" },
  };
  const applyMode = (m) => {
    set({ mode: m });
    toast(`${modes[m].label} 모드 적용 완료`);
  };
  const launch = () => {
    if (state.gameRunning) { toast("이미 실행 중입니다"); return; }
    set({ gameRunning: true });
    toast("검은사막 런처를 실행합니다");
  };

  const nextRun = state.rules.length
    ? `다음 ${state.rules[0].name || "규칙"} 대기 중`
    : "규칙 없음";

  return (
    <div className="tab-panel ctrl-tab">
      {/* status cards */}
      <div className="stat-grid" style={{ marginBottom: 14 }}>
        <div className="glass stat-card">
          <div className="stat-top"><span className="stat-dot" style={{ background: "var(--ok)" }}></span>권한</div>
          <div className="stat-val" style={{ color: "var(--ok)" }}>획득</div>
          <div className="stat-meta">관리자 권한 활성</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top">
            <span className="stat-dot" style={{ background: state.gameRunning ? "var(--ok)" : "var(--txt-faint)" }}></span>게임
          </div>
          <div className="stat-val" style={{ color: state.gameRunning ? "var(--ok)" : "var(--txt-dim)" }}>
            {state.gameRunning ? "실행 중" : "중지됨"}
          </div>
          <div className="stat-meta">BlackDesert64.exe</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top"><span className="stat-dot" style={{ background: "var(--accent)" }}></span>모드</div>
          <div className="stat-val" style={{ color: "var(--accent)" }}>{modes[state.mode].label} 모드</div>
          <div className="stat-meta">{modes[state.mode].badge}</div>
        </div>
        <div className="glass stat-card">
          <div className="stat-top"><span className="stat-dot" style={{ background: "var(--info)" }}></span>스케줄</div>
          <div className="stat-val" style={{ color: "var(--info)" }}>{state.rules.length ? `${state.rules.length}개 규칙` : "규칙 없음"}</div>
          <div className="stat-meta">{state.reservation ? "매주 수 05:00 (15시간 19분 남음)" : nextRun}</div>
        </div>
      </div>

      {/* GAME */}
      <div className="glass panel">
        <div className="section-label">
          <I.play size={13} /> Game
          <span className="help" title="런처 실행과 프로세스 상태를 관리합니다">?</span>
        </div>
        <p className="panel-sub">런처 실행과 프로세스 상태를 관리합니다.</p>
        <div className="row">
          <button className="btn" onClick={() => toast("프로세스 상태를 새로고침했습니다")}>
            <I.refresh size={15} /> 상태 새로고침
          </button>
          <button className="btn btn-primary" onClick={launch}>
            <I.play size={14} /> 게임 실행
          </button>
        </div>
      </div>

      {/* MODE */}
      <div className="glass panel">
        <div className="section-label">
          <I.gauge size={13} /> Mode
          <span className="help" title="우선순위와 CPU affinity를 즉시 적용합니다">?</span>
        </div>
        <p className="panel-sub">BlackDesert64.exe 우선순위와 CPU affinity를 즉시 적용합니다.</p>
        <div className="seg">
          {Object.entries(modes).map(([k, m]) => (
            <button key={k} className={"seg-btn" + (state.mode === k ? " on" : "")} onClick={() => applyMode(k)}>
              {state.mode === k && <span className="seg-badge">적용 중</span>}
              <m.icon className="ico" />
              {m.label}
            </button>
          ))}
        </div>
        <p className="legend">
          <b>고성능</b> 물리 코어 + High 우선순위 &nbsp;·&nbsp; <b>일반</b> 전체 코어 + Normal &nbsp;·&nbsp; <b>저전력</b> 마지막 코어 + Idle
        </p>
      </div>
    </div>
  );
}
window.ControlTab = ControlTab;
