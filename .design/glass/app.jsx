// app.jsx — window shell, tab nav, tweaks
const { useState, useRef, useEffect, useLayoutEffect } = React;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "mode": "dark",
  "bg": "aurora",
  "accent": ["#25d0c0", "#18a4e0", "#f0c04a"],
  "blur": 30,
  "frost": 0.12,
  "radius": 22
}/*EDITMODE-END*/;

const BG_BY_MODE = { dark: "aurora", light: "frost" };

// --- color helpers: derive a harmonious 3-stop accent palette from one hex ---
function hexToHsl(hex) {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map(c => c + c).join("");
  const r = parseInt(h.slice(0, 2), 16) / 255;
  const g = parseInt(h.slice(2, 4), 16) / 255;
  const b = parseInt(h.slice(4, 6), 16) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let hue = 0, s = 0, l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) hue = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) hue = (b - r) / d + 2;
    else hue = (r - g) / d + 4;
    hue *= 60;
  }
  return { h: hue, s: s * 100, l: l * 100 };
}
function hslToHex(h, s, l) {
  h = ((h % 360) + 360) % 360; s = Math.max(0, Math.min(100, s)) / 100; l = Math.max(0, Math.min(100, l)) / 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const to = v => Math.round((v + m) * 255).toString(16).padStart(2, "0");
  return "#" + to(r) + to(g) + to(b);
}
function paletteFromHex(hex) {
  const { h, s, l } = hexToHsl(hex);
  const sat = Math.max(45, Math.min(92, s));
  const primary = hslToHex(h, sat, Math.max(48, Math.min(64, l)));
  const secondary = hslToHex(h + 24, sat, Math.max(46, Math.min(60, l - 2)));   // analogous cooler/deeper
  const highlight = hslToHex(h - 38, Math.min(95, sat + 6), Math.min(72, l + 16)); // warm lighter pop
  return [primary, secondary, highlight];
}

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [tab, setTab] = useState(0);
  const [toastData, setToastData] = useState({ msg: "", show: false });
  const toastTimer = useRef(null);

  const [state, setState] = useState({
    gameRunning: true,
    mode: "low",
    rules: [],
    reservation: { type: "weekly", hh: 5, mm: 0 },
    settings: {
      themeChoice: "dark", reduceMotion: false,
      autoLowOnHide: true, closeToTray: true, autoOpt: true, notify: false,
      autostart: true, startInTray: true,
      dynAffinity: true, defaultMode: "저전력", interval: "0.5초",
      launcherPath: "",
    },
  });
  const set = (patch) => setState(s => ({ ...s, ...patch }));

  // theme: light / dark / auto(OS)
  const applyTheme = (choice) => {
    setState(s => ({ ...s, settings: { ...s.settings, themeChoice: choice } }));
    if (choice !== "auto") setTweak({ mode: choice, bg: BG_BY_MODE[choice] });
  };
  useEffect(() => {
    if (state.settings.themeChoice !== "auto") return;
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    const resolve = () => { const eff = mql.matches ? "dark" : "light"; setTweak({ mode: eff, bg: BG_BY_MODE[eff] }); };
    resolve();
    mql.addEventListener("change", resolve);
    return () => mql.removeEventListener("change", resolve);
  }, [state.settings.themeChoice]);

  // reduce motion
  useEffect(() => {
    document.body.classList.toggle("reduce-motion", state.settings.reduceMotion);
  }, [state.settings.reduceMotion]);
  const toast = (msg) => {
    setToastData({ msg, show: true });
    clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToastData(d => ({ ...d, show: false })), 2600);
  };

  // apply tweak vars to <body>
  useEffect(() => {
    const b = document.body;
    b.dataset.mode = t.mode;
    b.dataset.bg = t.bg === "auto" ? BG_BY_MODE[t.mode] : t.bg;
    const acc = Array.isArray(t.accent) ? t.accent : [t.accent, t.accent, t.accent];
    b.style.setProperty("--accent", acc[0]);
    b.style.setProperty("--accent-2", acc[1] || acc[0]);
    b.style.setProperty("--accent-3", acc[2] || acc[0]);
    b.style.setProperty("--blur", t.blur + "px");
    b.style.setProperty("--frost", t.frost);
    b.style.setProperty("--radius", t.radius + "px");
  }, [t]);

  // sliding tab pill
  const tabRefs = useRef([]);
  const [pill, setPill] = useState({ left: 0, width: 0 });
  const recalc = () => {
    const el = tabRefs.current[tab];
    if (el) setPill({ left: el.offsetLeft, width: el.offsetWidth });
  };
  useLayoutEffect(recalc, [tab]);
  useEffect(() => {
    const r = () => recalc();
    window.addEventListener("resize", r);
    const id = setTimeout(recalc, 60);
    return () => { window.removeEventListener("resize", r); clearTimeout(id); };
  }, []);

  const tabs = [
    { label: "제어", icon: I.control, comp: ControlTab },
    { label: "스케줄", icon: I.schedule, comp: ScheduleTab },
    { label: "모니터", icon: I.monitor, comp: MonitorTab },
    { label: "설정", icon: I.settings, comp: SettingsTab },
  ];
  const Active = tabs[tab].comp;

  const BG_OPTS = ["aurora", "dusk", "mesh", "frost"];
  const accArr = Array.isArray(t.accent) ? t.accent : [t.accent, t.accent, t.accent];
  const ACCENTS = [
    ["#25d0c0", "#18a4e0", "#f0c04a"], // teal (brand)
    ["#f0b53c", "#ff7a3d", "#ffd98a"], // BDO gold
    ["#7b8cff", "#5ad1ff", "#c4b6ff"], // indigo
    ["#ff6ea8", "#9b5cff", "#ffc4dd"], // magenta
  ];

  return (
    <>
      <div className="bg-stage">
        <div className="orb orb-1"></div><div className="orb orb-2"></div>
        <div className="orb orb-3"></div><div className="orb orb-4"></div>
      </div>
      <div className="bg-grain"></div>

      <div className="glass app-window">
        <div className="titlebar">
          <div className="title-id">
            <div className="app-mark"><I.bolt size={17} fill="#04181b" /></div>
            <div className="app-title">BDO Optimizer <span>· {state.mode === "low" ? "저전력" : state.mode === "high" ? "고성능" : "일반"}</span></div>
          </div>
          <div className="win-ctrls">
            <button className="win-dot min"></button>
            <button className="win-dot max"></button>
            <button className="win-dot close"></button>
          </div>
        </div>

        <div className="tabbar">
          <div className="tab-pill" style={{ left: pill.left, width: pill.width }}></div>
          {tabs.map((tb, i) => (
            <button key={i} ref={el => tabRefs.current[i] = el}
                    className={"tab" + (tab === i ? " active" : "")} onClick={() => setTab(i)}>
              <tb.icon /> {tb.label}
            </button>
          ))}
        </div>

        <div className="content" key={tab}>
          <Active state={state} set={set} toast={toast} applyTheme={applyTheme}
                  accent={t.accent} onAccent={v => setTweak("accent", v)} accents={ACCENTS} />
        </div>

        <div className="statusbar">
          <span className="status-led"></span>
          {state.mode === "low" ? "저전력 모드 적용 완료" : state.mode === "high" ? "고성능 모드 적용 완료" : "일반 모드 적용 완료"}
          <span style={{ marginLeft: "auto", color: "var(--txt-faint)" }}>{state.gameRunning ? "게임 실행 중" : "대기 중"}</span>
        </div>

        <div className={"toast" + (toastData.show ? " show" : "")}>
          <span className="status-led"></span>{toastData.msg}
        </div>
      </div>

      <TweaksPanel>
        <TweakSection label="테마" />
        <TweakRadio label="모드" value={t.mode} options={["dark", "light"]}
                    onChange={v => applyTheme(v)} />
        <TweakSelect label="배경" value={t.bg}
                     options={[
                       { value: "aurora", label: "오로라 (다크)" },
                       { value: "dusk", label: "데저트 더스크 (다크)" },
                       { value: "mesh", label: "메쉬 블루 (다크)" },
                       { value: "frost", label: "프로스트 (라이트)" },
                     ]}
                     onChange={v => setTweak("bg", v)} />
        <TweakColor label="액센트" value={t.accent} options={ACCENTS}
                    onChange={v => setTweak("accent", v)} />
        <TweakRow label="직접 선택">
          <label style={{ display: "flex", alignItems: "center", gap: 10, width: "100%", cursor: "pointer" }}>
            <span style={{ position: "relative", width: 32, height: 32, borderRadius: 9, overflow: "hidden",
                           background: accArr[0], boxShadow: "inset 0 0 0 1px rgba(0,0,0,.18), 0 1px 3px rgba(0,0,0,.2)", flexShrink: 0 }}>
              <input type="color" value={accArr[0]}
                     onChange={e => setTweak("accent", paletteFromHex(e.target.value))}
                     style={{ position: "absolute", inset: "-6px", width: "150%", height: "150%", border: 0, padding: 0, opacity: 0, cursor: "pointer" }} />
            </span>
            <span style={{ display: "flex", gap: 4 }} title="자동 생성된 보조 · 하이라이트">
              <i style={{ width: 16, height: 32, borderRadius: 5, background: accArr[1], boxShadow: "inset 0 0 0 1px rgba(0,0,0,.15)" }}></i>
              <i style={{ width: 16, height: 32, borderRadius: 5, background: accArr[2], boxShadow: "inset 0 0 0 1px rgba(0,0,0,.15)" }}></i>
            </span>
            <code style={{ marginLeft: "auto", fontSize: 11, letterSpacing: "0.02em", fontFamily: "ui-monospace, monospace", opacity: 0.75 }}>
              {String(accArr[0]).toUpperCase()}
            </code>
          </label>
        </TweakRow>
        <TweakSection label="글래스" />
        <TweakSlider label="블러" value={t.blur} min={6} max={60} step={2} unit="px"
                     onChange={v => setTweak("blur", v)} />
        <TweakSlider label="프로스트(불투명)" value={t.frost} min={0.03} max={0.3} step={0.01}
                     onChange={v => setTweak("frost", v)} />
        <TweakSlider label="모서리 둥글기" value={t.radius} min={8} max={34} step={1} unit="px"
                     onChange={v => setTweak("radius", v)} />
      </TweaksPanel>
    </>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
