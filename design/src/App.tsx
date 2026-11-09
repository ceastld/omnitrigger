import { useEffect, useState, type CSSProperties } from "react";
import fixture from "../data/ui-browse.json";
import type { Preview } from "./data";
import { TriggerPage } from "./TriggerPage";
import { PreviewContext } from "./ui";

const FORMAT = 1;

function load(value: unknown): Preview {
  const preview = value as Preview;
  if (preview?.format !== FORMAT) throw new Error(`预览数据格式 ${String(preview?.format)} 不受支持，请重新导出。`);
  return preview;
}

// 地址参数便于截图和对照：?view=keyboard&theme=light&width=800
const params = new URLSearchParams(location.search);
const initialView = (["list", "keyboard", "mouse"] as const).find((view) => view === params.get("view")) ?? "list";

export function App() {
  const [preview, setPreview] = useState<Preview>(() => load(fixture));
  const [source, setSource] = useState("仓库夹具 ui-browse");
  const [dark, setDark] = useState(params.get("theme") !== "light");
  const [width, setWidth] = useState(Number(params.get("width")) || 0);
  const [error, setError] = useState<string | null>(null);

  const palette = dark ? preview.tokens.dark : preview.tokens.light;
  const vars = Object.fromEntries(Object.entries(palette).map(([name, color]) => [`--${name}`, color]));
  useEffect(() => {
    document.body.style.background = palette.BACKGROUND;
  }, [palette.BACKGROUND]);

  const open = async (file: File | undefined) => {
    if (!file) return;
    try {
      setPreview(load(JSON.parse(await file.text())));
      setSource(file.name);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const style = {
    ...vars,
    "--mono-font": `"${preview.tokens.mono_font}", ui-monospace, monospace`,
    fontFamily: `"${preview.tokens.ui_font}", system-ui, sans-serif`,
    color: palette.TEXT,
    colorScheme: dark ? "dark" : "light",
    height: "100vh",
    overflow: "hidden",
    display: "flex",
    flexDirection: "column",
  } as CSSProperties;

  return (
    <PreviewContext.Provider value={preview}>
      <div style={style}>
        <div className="devbar">
          <span>数据：{source} · {preview.rows.length} 条</span>
          <label className="devbar-button">
            打开导出文件
            <input type="file" accept=".json" hidden onChange={(event) => open(event.target.files?.[0])} />
          </label>
          <button className="devbar-button" onClick={() => setDark(!dark)}>
            {dark ? "浅色" : "深色"}
          </button>
          <label>
            中栏宽度
            <select className="devbar-button" value={width} onChange={(event) => setWidth(Number(event.target.value))}>
              <option value={0}>填满窗口</option>
              {[560, 660, 760, 900].map((value) => (
                <option key={value} value={value}>
                  {value}px
                </option>
              ))}
            </select>
          </label>
          {error && <span className="devbar-error">{error}</span>}
        </div>
        <div style={{ flex: 1, minHeight: 0, display: "flex", background: palette.BACKGROUND }}>
          <div
            style={{
              flex: width === 0 ? 1 : "none",
              width: width === 0 ? undefined : width,
              minWidth: 0,
              display: "flex",
              borderRight: width === 0 ? undefined : `1px solid ${palette.DIVIDER}`,
            }}
          >
            {/* 切换数据时重建页面状态。 */}
            <TriggerPage key={source} initialView={initialView} />
          </div>
        </div>
      </div>
    </PreviewContext.Provider>
  );
}
