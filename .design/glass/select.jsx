// select.jsx — glassmorphic custom dropdown (replaces native <select>)
function GlassSelect({ value, options, onChange, width }) {
  const { I } = window;
  const [open, setOpen] = React.useState(false);
  const [pos, setPos] = React.useState({ left: 0, top: 0, width: 0, up: false });
  const wrapRef = React.useRef(null);
  const menuRef = React.useRef(null);
  const opts = options.map(o => (typeof o === "object" ? o : { value: o, label: o }));
  const cur = opts.find(o => o.value === value);

  const openMenu = () => {
    const r = wrapRef.current.getBoundingClientRect();
    const menuH = Math.min(opts.length * 42 + 12, 260);
    const up = r.bottom + menuH + 10 > window.innerHeight;
    setPos({ left: r.left, top: up ? r.top - menuH - 6 : r.bottom + 6, width: r.width, up });
    setOpen(true);
  };

  React.useEffect(() => {
    if (!open) return;
    const outside = (e) => {
      if (wrapRef.current && wrapRef.current.contains(e.target)) return;
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      setOpen(false);
    };
    const onScroll = () => setOpen(false);
    const onKey = (e) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open]);

  return (
    <div className="gsel" style={width ? { width, flex: "none" } : null} ref={wrapRef}>
      <button type="button" className={"field gsel-btn" + (open ? " open" : "")}
              onClick={() => (open ? setOpen(false) : openMenu())}>
        <span className="gsel-val">{cur ? cur.label : ""}</span>
        <I.down className="gsel-chev" size={14} />
      </button>
      {open && ReactDOM.createPortal(
        <div ref={menuRef} className={"gsel-menu" + (pos.up ? " up" : "")}
             style={{ left: pos.left, top: pos.top, minWidth: pos.width }}>
          {opts.map(o => (
            <button type="button" key={o.value}
                    className={"gsel-opt" + (o.value === value ? " on" : "")}
                    onClick={() => { onChange(o.value); setOpen(false); }}>
              <span>{o.label}</span>
              {o.value === value && <I.check size={14} />}
            </button>
          ))}
        </div>,
        document.body
      )}
    </div>
  );
}
window.GlassSelect = GlassSelect;

// PillSwitch — segmented toggle with a sliding indicator (like the tab pill)
function PillSwitch({ value, options, onChange, style, className }) {
  const refs = React.useRef([]);
  const [pill, setPill] = React.useState({ left: 4, width: 0 });
  const opts = options.map(o => (typeof o === "object" ? o : { value: o, label: o }));
  const idx = Math.max(0, opts.findIndex(o => o.value === value));

  const recalc = React.useCallback(() => {
    const el = refs.current[idx];
    if (el) setPill({ left: el.offsetLeft, width: el.offsetWidth });
  }, [idx]);

  React.useLayoutEffect(() => { recalc(); }, [recalc, opts.length]);
  React.useEffect(() => {
    const r = () => recalc();
    window.addEventListener("resize", r);
    const t = setTimeout(recalc, 70);
    return () => { window.removeEventListener("resize", r); clearTimeout(t); };
  }, [recalc]);

  return (
    <div className={"pillswitch" + (className ? " " + className : "")} style={style}>
      <span className="pill-ind" style={{ left: pill.left, width: pill.width }}></span>
      {opts.map((o, i) => (
        <button key={o.value} ref={el => (refs.current[i] = el)}
                className={o.value === value ? "on" : ""} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
window.PillSwitch = PillSwitch;
