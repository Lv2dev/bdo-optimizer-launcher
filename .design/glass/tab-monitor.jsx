// tab-monitor.jsx — 모니터 (Monitor) with live animated graphs
function smoothPath(vals, w, h, maxVal) {
  const n = vals.length;
  if (n < 2) return { line: "", area: "" };
  const pts = vals.map((v, i) => [ (i / (n - 1)) * w, h - (Math.min(v, maxVal) / maxVal) * (h - 6) - 3 ]);
  let line = `M ${pts[0][0]} ${pts[0][1]}`;
  for (let i = 0; i < n - 1; i++) {
    const [x0, y0] = pts[i], [x1, y1] = pts[i + 1];
    const cx = (x0 + x1) / 2;
    line += ` C ${cx} ${y0} ${cx} ${y1} ${x1} ${y1}`;
  }
  const area = line + ` L ${w} ${h} L 0 ${h} Z`;
  return { line, area };
}

function LiveGraph({ name, color, data, max, fmt, foot }) {
  const W = 320, H = 76;
  const { line, area } = smoothPath(data, W, H, max);
  const cur = data[data.length - 1] || 0;
  const gid = "g_" + name.replace(/[^a-z]/gi, "");
  return (
    <div className="glass graph-card">
      <div className="graph-head">
        <div className="graph-name" style={{ color }}>{name}</div>
        <div className="graph-val" style={{ color }}>{fmt(cur)}</div>
      </div>
      <svg className="graph-svg" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        <defs>
          <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.42" />
            <stop offset="100%" stopColor={color} stopOpacity="0.02" />
          </linearGradient>
        </defs>
        {[0.25, 0.5, 0.75].map(g => (
          <line key={g} x1="0" y1={H * g} x2={W} y2={H * g} stroke="var(--glass-stroke)" strokeWidth="0.5" />
        ))}
        <path d={area} fill={`url(#${gid})`} style={{ transition: "d 0.5s linear" }} />
        <path d={line} fill="none" stroke={color} strokeWidth="2" strokeLinecap="round"
              style={{ transition: "d 0.5s linear", filter: `drop-shadow(0 0 6px ${color}66)` }} />
        <circle cx={W} cy={H - (Math.min(cur, max) / max) * (H - 6) - 3} r="3.2" fill={color}
                style={{ filter: `drop-shadow(0 0 5px ${color})` }} />
      </svg>
      <div className="graph-foot"><span>{foot[0]}</span><span>{foot[1]}</span></div>
    </div>
  );
}

function MonitorTab() {
  const { I } = window;
  const [view, setView] = React.useState("bars"); // bars | cores
  const N = 40;
  const seed = (base, jit) => Array.from({ length: N }, () => base + (Math.random() - 0.5) * jit);
  const [cpu, setCpu] = React.useState(() => seed(4, 3));
  const [mem, setMem] = React.useState(() => seed(1.2, 0.1));
  const [gpu, setGpu] = React.useState(() => seed(2, 2));
  const [vram, setVram] = React.useState(() => seed(3900, 200));
  const [cores, setCores] = React.useState(() => Array.from({ length: 8 }, () => 5 + Math.random() * 10));

  React.useEffect(() => {
    const push = (arr, v) => [...arr.slice(1), v];
    const id = setInterval(() => {
      const spike = Math.random() > 0.85;
      setCpu(a => push(a, Math.max(0.5, Math.min(100, a[a.length-1] + (Math.random()-0.5)*4 + (spike?12:0)))));
      setMem(a => push(a, Math.max(0.8, Math.min(61.4, a[a.length-1] + (Math.random()-0.5)*0.15))));
      setGpu(a => push(a, Math.max(0.3, Math.min(100, a[a.length-1] + (Math.random()-0.5)*3 + (spike?8:0)))));
      setVram(a => push(a, Math.max(3000, Math.min(15977, a[a.length-1] + (Math.random()-0.5)*120))));
      setCores(cs => cs.map(c => Math.max(2, Math.min(100, c + (Math.random()-0.5)*14 + (spike?20:0)))));
    }, 600);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="tab-panel">
      {/* hardware */}
      <div className="glass panel" style={{ paddingTop: 14, paddingBottom: 14 }}>
        <div className="hw-row">
          <I.cpu size={18} style={{ color: "var(--accent)" }} />
          <span className="hw-key">CPU</span>
          <span className="hw-val">AMD Ryzen 7 9800X3D 8-Core Processor</span>
        </div>
        <div className="hw-row">
          <I.gpu size={18} style={{ color: "var(--accent-2)" }} />
          <span className="hw-key">GPU</span>
          <span className="hw-val">NVIDIA GeForce RTX 5080</span>
        </div>
      </div>

      {/* resource monitor */}
      <div className="glass panel">
        <div className="panel-head">
          <I.sparkle size={16} style={{ color: "var(--accent-3)" }} />
          <h3>검은사막 자원 모니터</h3>
          <PillSwitch value={view} onChange={setView} style={{ marginLeft: "auto", width: 168 }}
                      options={[
                        { value: "bars", label: <><I.bars size={13} style={{ verticalAlign: -2, marginRight: 5 }} />그래프</> },
                        { value: "cores", label: <><I.grid size={13} style={{ verticalAlign: -2, marginRight: 5 }} />코어별</> },
                      ]} />
        </div>
        <p className="panel-sub">BlackDesert64.exe 프로세스가 사용 중인 자원량입니다.</p>

        {view === "bars" ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <LiveGraph name="CPU"    color="#3fd0ff" data={cpu}  max={100}   fmt={v => `${v.toFixed(0)}%`} foot={["0 – 100 %", "30초"]} />
            <LiveGraph name="메모리" color="#b48cff" data={mem}  max={61.4}  fmt={v => <>{v.toFixed(1)} <small>/ 61.4 GB</small></>} foot={["사용 / 총", "30초"]} />
            <LiveGraph name="GPU"    color="#34e0a1" data={gpu}  max={100}   fmt={v => `${v.toFixed(0)}%`} foot={["0 – 100 %", "30초"]} />
            <LiveGraph name="VRAM"   color="#ff9b6e" data={vram} max={15977} fmt={v => <>{v.toFixed(0)} <small>/ 15977 MB</small></>} foot={["사용 / 총", "30초"]} />
          </div>
        ) : (
          <div>
            <div className="core-grid">
              {cores.map((c, i) => (
                <div key={i} className="core-cell">
                  <div className="ci">CORE {i}</div>
                  <div className="cv" style={{ color: c > 60 ? "var(--warn)" : "var(--accent)" }}>{c.toFixed(0)}%</div>
                  <div className="core-bar"><i style={{ width: `${c}%` }}></i></div>
                </div>
              ))}
            </div>
            <p className="legend" style={{ marginTop: 14 }}>물리 코어 8개의 실시간 점유율 · <b>고성능 모드</b>에서 물리 코어에 우선 할당됩니다.</p>
          </div>
        )}
      </div>
    </div>
  );
}
window.MonitorTab = MonitorTab;
