// 鼠标视图：对应 main.rs 的 render_mouse_view。坐标与应用一致，改动时两边同步。
import { useState, type CSSProperties, type ReactNode } from "react";
import { mouseRows, partCount, type Row } from "./data";
import { Segment, SectionHeading, c, mono, panelStyle } from "./ui";

// 机身左上角与宽度；其余坐标都相对整幅图。
const MX = 115;
const MW = 170;

const CALLOUTS: [part: string, y: number, anchor: [number, number], left: boolean][] = [
  ["左键", 56, [MX + 40, 56], true],
  ["侧键 2", 145, [MX - 6, 145], true],
  ["侧键 1", 185, [MX - 6, 185], true],
  ["中键", 12, [MX + MW / 2, 22], false],
  ["滚轮 ↑", 40, [MX + MW / 2 + 9, 40], false],
  ["滚轮 ↓", 70, [MX + MW / 2 + 9, 70], false],
  ["右键", 100, [MX + 130, 100], false],
];

export function MouseView({ rows, renderRow }: { rows: Row[]; renderRow: (row: Row) => ReactNode }) {
  const [picked, setPicked] = useState<string | null>(null);
  const count = (part: string) => partCount(rows, part);
  const total = (part: string) => {
    const value = count(part);
    return value.terminal + value.prefix;
  };
  const select = (part: string) => setPicked(picked === part ? null : part);
  const fill = (part: string) => {
    const n = total(part);
    return n === 0 ? c("SURFACE_HI") : c("ACCENT", Math.min(0.16 + n * 0.06, 0.5));
  };
  const wheelFill = (part: string) => (total(part) > 0 ? c("ACCENT", 0.55) : c("BORDER_MID"));
  const zone = (part: string, style: CSSProperties) => (
    <div
      className="hover-zone"
      onClick={() => select(part)}
      style={{
        position: "absolute",
        boxSizing: "border-box",
        background: fill(part),
        border: picked === part ? `2px solid ${c("ACCENT")}` : "1px solid rgba(255,255,255,0.06)",
        cursor: "pointer",
        ...style,
      }}
    />
  );
  const picks = picked === null ? null : count(picked);
  const listed = mouseRows(rows, picked);

  return (
    <>
      <div style={{ ...panelStyle, display: "flex", flexDirection: "column", alignItems: "center", gap: 12 }}>
        <div style={{ position: "relative", flex: "none", width: 410, height: 266 }}>
          <div
            style={{
              position: "absolute",
              left: MX,
              top: 0,
              width: MW,
              height: 262,
              boxSizing: "border-box",
              background: c("KEYCAP_BASE"),
              border: `1px solid ${c("BORDER_MID")}`,
              borderRadius: "84px 84px 76px 76px",
              boxShadow: "0 12px 32px -6px rgba(0,0,0,0.35)",
            }}
          />
          {zone("左键", { left: MX + 2, top: 2, width: MW / 2 - 4, height: 110, borderRadius: "82px 3px 3px 3px" })}
          {zone("右键", { left: MX + MW / 2 + 2, top: 2, width: MW / 2 - 4, height: 110, borderRadius: "3px 82px 3px 3px" })}
          {zone("中键", {
            left: MX + MW / 2 - 14,
            top: 22,
            width: 28,
            height: 66,
            borderRadius: 14,
            ...(total("中键") === 0 ? { background: c("SIDEBAR") } : {}),
          })}
          {zone("滚轮 ↑", { left: MX + MW / 2 - 9, top: 27, width: 18, height: 27, borderRadius: "9px 9px 0 0", background: wheelFill("滚轮 ↑") })}
          {zone("滚轮 ↓", { left: MX + MW / 2 - 9, top: 56, width: 18, height: 27, borderRadius: "0 0 9px 9px", background: wheelFill("滚轮 ↓") })}
          {zone("侧键 2", { left: MX - 6, top: 128, width: 10, height: 34, borderRadius: 4 })}
          {zone("侧键 1", { left: MX - 6, top: 168, width: 10, height: 34, borderRadius: 4 })}
          {CALLOUTS.map(([part, y, [ax, ay], left]) => {
            const n = total(part);
            const active = picked === part;
            const line = active ? c("ACCENT") : n > 0 ? c("ACCENT", 0.45) : c("BORDER_MID");
            const [from, to] = left ? [96, ax] : [ax, 312];
            return (
              <div key={part} style={{ display: "contents" }}>
                <div style={{ position: "absolute", left: from, top: y, width: to - from, height: 1, background: line }} />
                {ay !== y && (
                  <div style={{ position: "absolute", left: ax, top: Math.min(y, ay), width: 1, height: Math.abs(ay - y), background: line }} />
                )}
                <div style={{ position: "absolute", left: ax - 2.5, top: ay - 2.5, width: 5, height: 5, borderRadius: 3, background: line }} />
                <div
                  onClick={() => select(part)}
                  style={{
                    position: "absolute",
                    top: y - 10,
                    height: 20,
                    left: left ? 0 : 316,
                    width: left ? 92 : 94,
                    justifyContent: left ? "flex-end" : undefined,
                    display: "flex",
                    alignItems: "center",
                    gap: 6,
                    fontSize: 12,
                    whiteSpace: "nowrap",
                    cursor: "pointer",
                    color: active ? c("ACCENT") : n > 0 ? c("TEXT_SOFT") : c("TEXT_FAINT"),
                  }}
                >
                  {part}
                  {n > 0 && <span style={{ ...mono, fontWeight: 600, color: c("ACCENT") }}>{n}</span>}
                </div>
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {["横向滚轮", "屏幕区域"]
            .filter((part) => total(part) > 0 || picked === part)
            .map((part) => (
              <Segment key={part} label={`${part} ${total(part)}`} active={picked === part} onClick={() => select(part)} />
            ))}
        </div>
      </div>
      <SectionHeading
        title={picked && picks ? `${picked}：作目标 ${picks.terminal} · 作前缀 ${picks.prefix}` : "所有用到鼠标、滚轮或屏幕区域的触发器"}
        count={listed.length}
      />
      {listed.map(renderRow)}
    </>
  );
}
