// 键盘视图：对应 main.rs 的 render_keyboard_view。键位与每条触发器点亮的键由应用导出。
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { keyLayers, keyboardView, type Row } from "./data";
import { Caption, SectionHeading, c, mono, panelStyle, usePreview } from "./ui";

export function KeyboardView({ rows, renderRow }: { rows: Row[]; renderRow: (row: Row) => ReactNode }) {
  const { keyboard } = usePreview();
  const [layerChoice, setLayer] = useState<string | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const probe = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const element = probe.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  // 键宽按面板可用宽度计算，与应用相同的上下限。
  const unit = width > 0 ? Math.min(40, Math.max(22, width / keyboard.width)) : 30;

  const layers = keyLayers(rows);
  const layer = layers.some(([name]) => name === layerChoice) ? layerChoice : null;
  const view = keyboardView(rows, layer, picked);
  const total = layers.reduce((sum, [, count]) => sum + count, 0);
  const pickedLabel = keyboard.slots.find((slot) => slot.vk === picked)?.label;
  const title = pickedLabel
    ? `目标键 ${pickedLabel}`
    : layer === null
      ? "所有以键盘按键结尾的触发器"
      : layer === ""
        ? "无前缀"
        : `按住 ${layer} 时`;

  const chip = (text: string, count: number, active: boolean, onClick: () => void) => (
    <div
      key={text}
      className={active ? undefined : "hover-border"}
      onClick={onClick}
      style={{
        height: 28,
        padding: "0 10px",
        boxSizing: "border-box",
        borderRadius: 6,
        display: "flex",
        alignItems: "center",
        gap: 6,
        fontSize: 12,
        whiteSpace: "nowrap",
        cursor: "pointer",
        ...(active
          ? { background: c("RECORD_CAP"), border: `1px solid ${c("RECORD_LINE")}`, color: c("RECORD") }
          : { background: c("SURFACE"), border: `1px solid ${c("BORDER")}`, color: c("TEXT_MUTED") }),
      }}
    >
      {text}
      <span style={{ ...mono, fontSize: 11, opacity: 0.75 }}>{count}</span>
    </div>
  );
  const choose = (next: string | null) => {
    setLayer(next);
    setPicked(null);
  };

  return (
    <>
      <div style={{ flex: "none", display: "flex", flexWrap: "wrap", alignItems: "center", gap: 6, paddingBottom: 10 }}>
        <div style={{ fontSize: 12, color: c("TEXT_FAINT") }}>按住</div>
        {chip("全部层", total, layer === null, () => choose(null))}
        {layers.map(([name, count]) => chip(name === "" ? "无前缀" : name, count, layer === name, () => choose(name)))}
      </div>
      <div style={{ ...panelStyle, display: "flex", flexDirection: "column", gap: 10 }}>
        <div ref={probe} style={{ width: "100%", height: 0 }} />
        <div style={{ position: "relative", flex: "none", width: unit * keyboard.width, height: unit * keyboard.height }}>
          {keyboard.slots.map((slot) => {
            const held = view.held.has(slot.vk);
            const bound = view.bound.has(slot.vk);
            const disabledOnly = view.boundDisabled.has(slot.vk);
            const isPicked = picked === slot.vk;
            const [bg, border, text] = held
              ? [c("RECORD_CAP"), c("RECORD_LINE"), c("RECORD")]
              : bound && !disabledOnly
                ? [c("ACCENT", 0.3), c("ACCENT", 0.6), c("TEXT")]
                : bound
                  ? [c("SURFACE_HI"), c("BORDER_HI"), c("TEXT_MUTED")]
                  : [c("KEYCAP"), c("KEYCAP_EDGE"), c("TEXT_FAINT")];
            const edge = isPicked ? c("ACCENT") : border;
            return (
              <div
                key={slot.vk}
                className={bound ? "hover-key" : undefined}
                onClick={bound ? () => setPicked(isPicked ? null : slot.vk) : undefined}
                style={{
                  position: "absolute",
                  left: slot.x * unit,
                  top: slot.y * unit,
                  width: slot.w * unit - 3,
                  height: unit - 3,
                  boxSizing: "border-box",
                  borderRadius: 5,
                  background: bg,
                  border: `${isPicked ? 2 : 1}px solid ${edge}`,
                  borderBottomWidth: 2,
                  opacity: layer !== null && !held && !bound ? 0.45 : 1,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  overflow: "hidden",
                  ...mono,
                  fontSize: slot.label.length > 3 ? Math.min(unit * 0.3, 11) : Math.min(unit * 0.36, 13),
                  color: text,
                  fontWeight: held ? 600 : undefined,
                  cursor: bound ? "pointer" : undefined,
                }}
              >
                {slot.label}
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 14, fontSize: 11, color: c("TEXT_FAINT") }}>
          <Legend color={c("RECORD_CAP")} border={c("RECORD_LINE")} text="按住的前缀" />
          <Legend color={c("ACCENT", 0.3)} border={c("ACCENT", 0.6)} text="已绑定" />
          <Legend color={c("SURFACE_HI")} border={c("BORDER_HI")} text="仅有停用绑定" />
          <span>点击已绑定的键查看触发器</span>
        </div>
      </div>
      <SectionHeading title={title} count={view.rows.length} />
      {view.offLayout > 0 && picked === null && (
        <Caption style={{ padding: "0 4px 4px" }}>其中 {view.offLayout} 条的目标键不在上方布局中，例如小键盘或媒体键。</Caption>
      )}
      {view.rows.map(renderRow)}
    </>
  );
}

function Legend({ color, border, text }: { color: string; border: string; text: string }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
      <div style={{ width: 11, height: 11, boxSizing: "border-box", borderRadius: 3, background: color, border: `1px solid ${border}` }} />
      {text}
    </div>
  );
}
