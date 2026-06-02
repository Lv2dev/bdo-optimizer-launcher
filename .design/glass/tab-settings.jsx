// tab-settings.jsx — 설정 (Settings) — mirrors the real launcher + keeps added features
function ToggleSwitch({ on, onToggle, label }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
      <button className={"switch" + (on ? " on" : "")} onClick={onToggle}><i></i></button>
      {label && <span style={{ fontSize: 12.5, color: "var(--txt-dim)", fontWeight: 600 }}>{label}</span>}
    </div>
  );
}

function SettingsTab({ state, set, toast, applyTheme, accent, onAccent, accents }) {
  const { I } = window;
  const s = state.settings;
  const upd = (patch) => set({ settings: { ...s, ...patch } });
  const toggle = (k) => upd({ [k]: !s[k] });

  const accArr = Array.isArray(accent) ? accent : [accent];
  const accNames = ["청록", "골드", "인디고", "마젠타"];

  const Help = ({ tip }) => <span className="help" title={tip}>?</span>;

  const themeOpts = [
    { v: "light", icon: I.sun,   label: "라이트 모드" },
    { v: "dark",  icon: I.moon,  label: "다크 모드" },
    { v: "auto",  icon: I.autoT, label: "자동 (OS 설정)", desc: "Windows 앱 테마 설정을 따릅니다." },
  ];

  return (
    <div className="tab-panel">
      {/* 테마 */}
      <div className="glass panel">
        <div className="set-head">테마<Help tip="앱 표시 색상을 변경합니다" /></div>
        <p className="panel-sub">앱 표시 색상을 변경합니다.</p>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {themeOpts.map(o => (
            <button key={o.v} className={"glass-2 theme-card" + (s.themeChoice === o.v ? " on" : "")}
                    onClick={() => { applyTheme(o.v); toast(`${o.label}로 변경했습니다`); }}>
              <o.icon className="tc-ico" />
              <div className="tc-body">
                <div className="tc-t">{o.label}</div>
                {o.desc && <div className="tc-d">{o.desc}</div>}
              </div>
              {s.themeChoice === o.v && <I.check className="tc-check" size={18} />}
            </button>
          ))}
        </div>

        <div style={{ height: 1, background: "var(--glass-stroke)", margin: "16px 0 14px" }}></div>
        <div className="set-head" style={{ fontSize: 12.5 }}>액센트 색상</div>
        <p className="panel-sub" style={{ marginBottom: 12 }}>버튼·그래프·강조 요소에 쓰이는 포인트 색입니다.</p>
        <div className="accent-grid">
          {(accents || []).map((pal, i) => {
            const on = String(accArr[0]).toLowerCase() === String(pal[0]).toLowerCase();
            return (
              <button key={i} className={"accent-swatch" + (on ? " on" : "")}
                      onClick={() => { onAccent(pal); toast(`${accNames[i]} 액센트 적용`); }} title={accNames[i]}>
                <span className="sw-colors">
                  {pal.slice(0, 3).map((c, j) => <i key={j} style={{ background: c }}></i>)}
                </span>
                <span className="sw-name">{accNames[i]}</span>
                {on && <I.check size={15} className="sw-check" />}
              </button>
            );
          })}
        </div>
      </div>

      {/* 접근성 */}
      <div className="glass panel">
        <div className="set-head"><I.eye size={15} style={{ opacity: 0.7 }} />접근성<Help tip="모션을 줄여 시각적 피로를 낮춥니다" /></div>
        <p className="panel-sub">애니메이션 줄이기를 켜면 신규 모션이 비활성화됩니다. 배터리 절약과 시각적 피로 감소에 도움이 됩니다.</p>
        <ToggleSwitch on={s.reduceMotion} onToggle={() => toggle("reduceMotion")} label={s.reduceMotion ? "켜짐" : "꺼짐"} />
      </div>

      {/* 런처 동작 */}
      <div className="glass panel">
        <div className="set-head">런처 동작<Help tip="게임/창 상태에 따른 자동 동작" /></div>
        <div className="set-row" style={{ paddingTop: 4 }}>
          <div className="meta"><div className="t">게임이 트레이로 내려가면 자동 저전력 모드</div><div className="d">검은사막 창이 숨겨지면 저전력 모드 적용, 다시 나타나면 직전 모드로 복원합니다.</div></div>
          <ToggleSwitch on={s.autoLowOnHide} onToggle={() => toggle("autoLowOnHide")} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t">창 닫기 시 트레이로 숨기기</div><div className="d">끄면 X 버튼 클릭 시 앱이 완전히 종료됩니다.</div></div>
          <ToggleSwitch on={s.closeToTray} onToggle={() => toggle("closeToTray")} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t">게임 실행 감지 시 자동 최적화</div><div className="d">BlackDesert64.exe 감지 시 마지막 모드를 적용합니다.</div></div>
          <ToggleSwitch on={s.autoOpt} onToggle={() => toggle("autoOpt")} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t">알림 표시</div><div className="d">모드 전환·예약 동작 시 데스크톱 알림을 보냅니다.</div></div>
          <ToggleSwitch on={s.notify} onToggle={() => toggle("notify")} />
        </div>
      </div>

      {/* 시작 옵션 */}
      <div className="glass panel">
        <div className="set-head">시작 옵션<Help tip="Windows 로그온 시 동작" /></div>
        <p className="panel-sub">Windows 로그온 시 자동으로 실행합니다. 작업 스케줄러에 등록되어 UAC 프롬프트 없이 승격 실행됩니다.</p>
        <div className="set-row" style={{ paddingTop: 4 }}>
          <div className="meta"><div className="t" style={{ color: s.autostart ? "var(--accent)" : "var(--txt)" }}>Windows 시작 시 자동 실행</div></div>
          <ToggleSwitch on={s.autostart} onToggle={() => toggle("autostart")} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t" style={{ color: s.startInTray ? "var(--accent)" : "var(--txt)" }}>자동 실행 시 트레이로 시작</div><div className="d">켜면 부팅 시 창을 띄우지 않고 트레이에만 상주합니다.</div></div>
          <ToggleSwitch on={s.startInTray} onToggle={() => toggle("startInTray")} />
        </div>
      </div>

      {/* 최적화 기본값 */}
      <div className="glass panel">
        <div className="set-head"><I.gauge size={15} style={{ opacity: 0.7 }} />최적화 기본값<Help tip="성능 모드 기본 동작" /></div>
        <div className="set-row" style={{ paddingTop: 4 }}>
          <div className="meta"><div className="t">기본 적용 모드</div><div className="d">앱 시작 시 자동으로 적용할 성능 모드입니다.</div></div>
          <GlassSelect value={s.defaultMode} width={120} options={["고성능", "일반", "저전력"]}
                       onChange={v => { upd({ defaultMode: v }); toast("기본 모드를 변경했습니다"); }} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t">CPU affinity 자동 조정</div><div className="d">코어 점유율에 따라 affinity를 동적으로 재배치합니다.</div></div>
          <ToggleSwitch on={s.dynAffinity} onToggle={() => toggle("dynAffinity")} />
        </div>
        <div className="set-row">
          <div className="meta"><div className="t">모니터 갱신 주기</div><div className="d">자원 그래프 샘플링 간격입니다.</div></div>
          <GlassSelect value={s.interval} width={100} options={["0.5초", "1초", "2초"]}
                       onChange={v => upd({ interval: v })} />
        </div>
      </div>

      {/* 런처 경로 */}
      <div className="glass panel">
        <div className="set-head"><I.folder size={15} style={{ opacity: 0.7 }} />런처 경로<Help tip="검은사막 실행 파일 경로" /></div>
        <p className="panel-sub">저장된 경로가 없으면 자동으로 탐색합니다.</p>
        <div className="path-field">
          <I.folder size={15} style={{ opacity: 0.6, flexShrink: 0 }} />
          <span className={s.launcherPath ? "mono" : ""}>{s.launcherPath || "저장된 경로 없음 (자동 탐색)"}</span>
        </div>
        <button className="btn btn-block" style={{ marginTop: 11 }}
                onClick={() => { upd({ launcherPath: "" }); toast("런처 경로를 초기화했습니다"); }}>
          <I.rotate size={15} /> 경로 초기화
        </button>
      </div>

      {/* 진단 */}
      <div className="glass panel">
        <div className="set-head"><I.bug size={15} style={{ opacity: 0.7 }} />진단<Help tip="로그 파일로 문제를 진단합니다" /></div>
        <p className="panel-sub">버그 보고 시 로그 파일을 첨부하면 진단에 도움이 됩니다.</p>
        <button className="btn btn-block" onClick={() => toast("로그 폴더를 엽니다")}>
          <I.folder size={15} /> 로그 폴더 열기
        </button>
      </div>

      <div className="app-footer">bdo-optimizer-launcher v0.1.0</div>
    </div>
  );
}
window.SettingsTab = SettingsTab;
