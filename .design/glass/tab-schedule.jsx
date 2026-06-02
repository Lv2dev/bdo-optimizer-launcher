// tab-schedule.jsx — 스케줄 (Schedule)
function ScheduleTab({ state, set, toast }) {
  const { I } = window;
  const [name, setName] = React.useState("");
  const [freq, setFreq] = React.useState("매일");
  const [start, setStart] = React.useState("");
  const [end, setEnd] = React.useState("");
  const [rmode, setRmode] = React.useState("고성능");

  const [resType, setResType] = React.useState("once"); // once | weekly
  const [hh, setHh] = React.useState(0);
  const [mm, setMm] = React.useState(0);

  const [openAuto, setOpenAuto] = React.useState(false);  // collapsed by default
  const [openRes, setOpenRes] = React.useState(true);     // expanded by default

  const addRule = () => {
    if (!start || !end) { toast("시작·종료 시간을 입력하세요"); return; }
    const rule = { id: Date.now(), name: name || "이름 없는 규칙", freq, start, end, mode: rmode };
    set({ rules: [...state.rules, rule] });
    setName(""); setStart(""); setEnd("");
    toast("자동 전환 규칙을 추가했습니다");
  };
  const delRule = (id) => set({ rules: state.rules.filter(r => r.id !== id) });

  const step = (cur, d, max) => (cur + d + max) % max;

  const register = () => {
    set({ reservation: { type: resType, hh, mm } });
    toast(`PC 예약 종료 등록 · ${String(hh).padStart(2,"0")}:${String(mm).padStart(2,"0")}`);
  };
  const cancel = () => { set({ reservation: null }); toast("예약을 취소했습니다"); };

  const modeColor = (m) => m === "고성능" ? "var(--warn)" : m === "저전력" ? "var(--ok)" : "var(--info)";

  return (
    <div className="tab-panel">
      {/* 자동 모드 전환 */}
      <div className="glass panel">
        <button className={"acc-head" + (openAuto ? " open" : "")} onClick={() => setOpenAuto(o => !o)}>
          <I.repeat size={17} />
          <h3>자동 모드 전환</h3>
          <span className="chip" style={{ marginLeft: "auto" }}>{state.rules.length ? `${state.rules.length}개 활성` : "규칙 없음"}</span>
          <I.down className="acc-chev" size={18} />
        </button>
        <div className={"acc-body" + (openAuto ? " open" : "")}>
          <div className="acc-inner">
            <div style={{ paddingTop: 14 }}>
              <p className="panel-sub" style={{ marginBottom: 14 }}>시간대별 자동 최적화 (앱 실행 중에만 동작)</p>
              <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                <input className="field" placeholder="규칙 이름 (예: 게임 시간 고성능)" value={name} onChange={e => setName(e.target.value)} />
                <GlassSelect value={freq} onChange={setFreq}
                             options={["매일", "평일", "주말", "월", "화", "수", "목", "금", "토", "일"]} />
                <div className="row">
                  <input className="field" placeholder="시작 HH:MM" value={start} onChange={e => setStart(e.target.value)} />
                  <input className="field" placeholder="종료 HH:MM" value={end} onChange={e => setEnd(e.target.value)} />
                  <GlassSelect value={rmode} onChange={setRmode} options={["고성능", "일반", "저전력"]} />
                </div>
                <button className="btn btn-primary btn-block" onClick={addRule}><I.plus size={15} /> 규칙 추가</button>
              </div>

              {state.rules.length === 0 ? (
                <p className="empty">아직 등록된 규칙이 없습니다.</p>
              ) : (
                <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 9 }}>
                  {state.rules.map(r => (
                    <div key={r.id} className="glass-2 rule-item" style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 14px" }}>
                      <span className="stat-dot" style={{ background: modeColor(r.mode) }}></span>
                      <div style={{ flex: 1 }}>
                        <div style={{ fontSize: 13, fontWeight: 600 }}>{r.name}</div>
                        <div style={{ fontSize: 11.5, color: "var(--txt-dim)", marginTop: 2 }}>{r.freq} · {r.start}–{r.end}</div>
                      </div>
                      <span className="chip accent">{r.mode}</span>
                      <button className="step-btn" onClick={() => delRule(r.id)} title="삭제"><I.trash size={14} /></button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* PC 예약 종료 */}
      <div className="glass panel">
        <button className={"acc-head" + (openRes ? " open" : "")} onClick={() => setOpenRes(o => !o)}>
          <I.power size={17} />
          <h3>PC 예약 종료</h3>
          <span className="chip warn" style={{ marginLeft: "auto" }}><I.warn size={12} /> 전원 차단</span>
          <I.down className="acc-chev" size={18} />
        </button>
        <div className={"acc-body" + (openRes ? " open" : "")}>
          <div className="acc-inner">
            <div style={{ paddingTop: 14 }}>
              {state.reservation ? (
                <p className="panel-sub" style={{ color: "var(--warn)", marginBottom: 14 }}>● 매주 수 05:00 (다음 15시간 19분 남음)</p>
              ) : (
                <p className="panel-sub" style={{ marginBottom: 14 }}>지정한 시간에 PC를 자동으로 종료합니다.</p>
              )}

              <PillSwitch value={resType} onChange={setResType} style={{ marginBottom: 14 }}
                          options={[{ value: "once", label: "단발 종료" }, { value: "weekly", label: "매주 반복" }]} />

              <button className="field" style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 16, cursor: "pointer" }}>
                <I.schedule size={15} /> <span style={{ color: "var(--accent)", fontWeight: 600 }}>2026-05-30 (토)</span>
                <I.down size={14} style={{ marginLeft: "auto", opacity: 0.6 }} />
              </button>

              <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: "0.08em", color: "var(--txt-faint)", marginBottom: 12 }}>종료 시간</div>
              <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 18, marginBottom: 18 }}>
                <div className="stepper">
                  <div className="lab">시</div>
                  <button className="step-btn" onClick={() => setHh(step(hh, 1, 24))}><I.up /></button>
                  <div className="num">{String(hh).padStart(2, "0")}</div>
                  <button className="step-btn" onClick={() => setHh(step(hh, -1, 24))}><I.down /></button>
                </div>
                <div style={{ fontSize: 28, fontWeight: 700, color: "var(--txt-faint)", paddingTop: 18 }}>:</div>
                <div className="stepper">
                  <div className="lab">분</div>
                  <button className="step-btn" onClick={() => setMm(step(mm, 5, 60))}><I.up /></button>
                  <div className="num">{String(mm).padStart(2, "0")}</div>
                  <button className="step-btn" onClick={() => setMm(step(mm, -5, 60))}><I.down /></button>
                </div>
              </div>

              <div className="row">
                <button className="btn btn-warn" onClick={register}><I.check size={15} /> 예약 등록</button>
                <button className="btn" onClick={cancel} disabled={!state.reservation} style={!state.reservation ? { opacity: 0.5, cursor: "not-allowed" } : {}}>예약 취소</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
window.ScheduleTab = ScheduleTab;
