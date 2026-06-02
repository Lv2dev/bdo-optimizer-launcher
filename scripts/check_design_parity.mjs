import { readFileSync } from "node:fs";

const main = readFileSync("web/src/main.jsx", "utf8");
const css = readFileSync("web/src/styles.css", "utf8");
let capabilities = "";
try {
  capabilities = readFileSync("capabilities/default.json", "utf8");
} catch {
  capabilities = "";
}
const controlTab = main.slice(main.indexOf("function ControlTab"), main.indexOf("function ScheduleTab"));
const settingsTab = main.slice(main.indexOf("function SettingsTab"), main.indexOf("function App"));

const checks = [
  {
    name: "app shell keeps design win-dot controls",
    pass: /className="win-dot min"/.test(main)
      && /className="win-dot max"/.test(main)
      && /className="win-dot close"/.test(main)
      && !/className="win-btn/.test(main),
  },
  {
    name: "control tab keeps design help affordances and no launcher path field",
    pass: main.includes('className="help"')
      && controlTab.includes("<Help")
      && main.includes("런처 실행과 프로세스 상태를 관리합니다.")
      && !controlTab.includes("path-field"),
  },
  {
    name: "schedule tab keeps design field and GlassSelect flow",
    pass: main.includes("function GlassSelect")
      && main.includes("<GlassSelect")
      && main.includes('className="field"')
      && main.includes("예약 취소"),
  },
  {
    name: "settings tab keeps vertical theme cards and accent swatches",
    pass: settingsTab.includes('className={"glass-2 theme-card"')
      && settingsTab.includes("accent-grid")
      && settingsTab.includes("accent-swatch")
      && settingsTab.includes('className="path-field readonly"')
      && settingsTab.includes("onAccent(palette)")
      && main.includes("앱 표시 색상을 변경합니다."),
  },
  {
    name: "css keeps design interaction primitives",
    pass: css.includes(".win-dot")
      && css.includes(".gsel-menu")
      && css.includes(".accent-swatch")
      && css.includes("body[data-bg=\"dusk\"]")
      && css.includes("body[data-bg=\"mesh\"]"),
  },
  {
    name: "runtime marks body for native frame layout",
    pass:
      /classList\.toggle\(["']tauri-runtime["'],\s*isTauriRuntime\(\)\)/.test(main)
      && /classList\.toggle\(["']native-app-surface["']/.test(main)
      && /dataset\.runtime\s*=\s*isTauriRuntime\(\)\s*\?/.test(main),
  },
  {
    name: "css removes nested app frame on native app surfaces",
    pass: /body\.tauri-runtime\s+#root/.test(css)
      && /body\.native-app-surface\s+#root/.test(css)
      && /body\.tauri-runtime\s+\.app-window[\s\S]*width:\s*100vw[\s\S]*height:\s*100vh[\s\S]*border-radius:\s*0[\s\S]*border:\s*0[\s\S]*box-shadow:\s*none/.test(css),
  },
  {
    name: "tauri capability grants window controls for the main window",
    pass: capabilities.includes('"core:window:allow-start-dragging"')
      && capabilities.includes('"core:window:allow-minimize"')
      && capabilities.includes('"core:window:allow-toggle-maximize"')
      && capabilities.includes('"core:window:allow-close"')
      && /"windows"\s*:\s*\[\s*"main"\s*\]/.test(capabilities),
  },
  {
    name: "css breakpoint does not collapse control grid at default window width",
    pass: !css.includes("max-width: 760px") && css.includes("max-width: 640px"),
  },
  {
    name: "css restores keyboard focus visibility on custom controls",
    pass: css.includes(":focus-visible") && /\.field:focus-visible/.test(css),
  },
];

const failed = checks.filter((check) => !check.pass);

if (failed.length > 0) {
  console.error("Design parity checks failed:");
  for (const check of failed) {
    console.error(`- ${check.name}`);
  }
  process.exit(1);
}

console.log(`Design parity checks passed (${checks.length}/${checks.length}).`);
