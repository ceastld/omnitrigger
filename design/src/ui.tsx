// 对应 crates/omnitrigger-app/src/theme.rs 的通用组件；尺寸与配色按应用取值，改这里前先看那边。
import { createContext, useContext, type CSSProperties, type ReactNode } from "react";
import type { Cap, Preview } from "./data";

export const PreviewContext = createContext<Preview | null>(null);

export function usePreview(): Preview {
  const preview = useContext(PreviewContext);
  if (!preview) throw new Error("缺少预览数据");
  return preview;
}

/** 语义色：`c("ACCENT")`，带透明度：`c("ACCENT", 0.3)`。 */
export function c(token: string, alpha?: number): string {
  return alpha === undefined
    ? `var(--${token})`
    : `color-mix(in srgb, var(--${token}) ${Math.round(alpha * 100)}%, transparent)`;
}

export const mono: CSSProperties = { fontFamily: "var(--mono-font)" };

export function Icon({ path, size, color }: { path: string; size: number; color: string }) {
  const icons = usePreview().icons;
  const svg = icons[path] ?? icons["icons/missing.svg"];
  const url = `url("data:image/svg+xml;utf8,${encodeURIComponent(svg)}")`;
  return (
    <span
      style={{
        flex: "none",
        width: size,
        height: size,
        background: color,
        maskImage: url,
        WebkitMaskImage: url,
        maskSize: "contain",
        WebkitMaskSize: "contain",
      }}
    />
  );
}

export type CapSize = "small" | "medium" | "large";
const CAP_METRICS: Record<CapSize, { height: number; radius: number; padding: number; font: number; icon: number }> = {
  small: { height: 18, radius: 4, padding: 6, font: 10.5, icon: 10 },
  medium: { height: 24, radius: 5, padding: 8, font: 12, icon: 12 },
  large: { height: 34, radius: 6, padding: 12, font: 14, icon: 15 },
};

export function Keycap({ cap, size }: { cap: Cap; size: CapSize }) {
  const m = CAP_METRICS[size];
  return (
    <div
      title={cap.title ?? undefined}
      style={{
        flex: "none",
        height: m.height,
        padding: `0 ${m.padding}px`,
        borderRadius: m.radius,
        background: c("KEYCAP"),
        border: `1px solid ${c("KEYCAP_EDGE")}`,
        display: "flex",
        alignItems: "center",
        gap: m.icon * 0.4,
        fontSize: m.font,
        fontWeight: 600,
        color: c("TEXT_SOFT"),
        whiteSpace: "nowrap",
        boxSizing: "border-box",
      }}
    >
      {cap.mouse && <Icon path="icons/mouse.svg" size={m.icon} color={c("ICON")} />}
      {cap.label}
      {cap.arrow && <Icon path={cap.arrow} size={m.icon} color={c("ACCENT")} />}
    </div>
  );
}

export function Combo({ caps, size = "small" }: { caps: Cap[]; size?: CapSize }) {
  const gap = size === "large" ? 9 : 5;
  const plus = { small: 11, medium: 12, large: 16 }[size];
  return (
    <div style={{ display: "flex", flex: "none", flexWrap: "nowrap", alignItems: "center", gap }}>
      {caps.map((cap, index) => (
        <span key={index} style={{ display: "contents" }}>
          {index > 0 && <span style={{ flex: "none", fontSize: plus, color: c("PLUS") }}>+</span>}
          <Keycap cap={cap} size={size} />
        </span>
      ))}
    </div>
  );
}

export function IconTile({ path, color, size }: { path: string; color: string; size: number }) {
  return (
    <div
      style={{
        flex: "none",
        width: size,
        height: size,
        borderRadius: size * 0.2,
        background: c(color, 0.1),
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Icon path={path} size={size * 0.48} color={c(color)} />
    </div>
  );
}

export function Switch({ on, onClick }: { on: boolean; onClick?: () => void }) {
  return (
    <div
      onClick={onClick}
      style={{
        flex: "none",
        position: "relative",
        width: 40,
        height: 22,
        borderRadius: 11,
        cursor: "pointer",
        boxSizing: "border-box",
        background: on ? c("ACCENT") : c("SURFACE_HI"),
        border: on ? "none" : `1px solid ${c("BORDER_MID")}`,
      }}
    >
      <div
        style={{
          position: "absolute",
          top: on ? 3 : 2,
          left: on ? 21 : 2,
          width: 16,
          height: 16,
          borderRadius: 8,
          background: on ? "#ffffff" : "#6b7287",
        }}
      />
    </div>
  );
}

export function Segments({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        flex: "none",
        display: "flex",
        padding: 3,
        gap: 3,
        borderRadius: 11,
        background: c("SIDEBAR"),
        border: `1px solid ${c("BORDER")}`,
      }}
    >
      {children}
    </div>
  );
}

export function Segment({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <div
      className={active ? undefined : "hover-segment"}
      onClick={onClick}
      style={{
        height: 30,
        padding: "0 12px",
        borderRadius: 8,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        fontSize: 12,
        whiteSpace: "nowrap",
        cursor: "pointer",
        boxSizing: "border-box",
        ...(active
          ? {
              background: c("SURFACE_HI"),
              border: `1px solid ${c("BORDER_HI")}`,
              color: c("TEXT"),
              fontWeight: 600,
            }
          : { color: c("TEXT_DIM"), border: "1px solid transparent" }),
      }}
    >
      {label}
    </div>
  );
}

export function Caption({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return <div style={{ fontSize: 11.5, color: c("TEXT_FAINT"), ...style }}>{children}</div>;
}

export function Heading({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
      <div style={{ fontSize: 20, lineHeight: "28px", fontWeight: 600 }}>{title}</div>
      <div style={{ fontSize: 12.5, color: c("TEXT_DIM") }}>{subtitle}</div>
    </div>
  );
}

export function PrimaryButton({ icon, label }: { icon: string; label: string }) {
  return (
    <div
      style={{
        flex: "none",
        height: 36,
        padding: "0 15px",
        borderRadius: 6,
        display: "flex",
        alignItems: "center",
        gap: 7,
        fontSize: 13,
        fontWeight: 600,
        color: c("ON_ACCENT"),
        background: c("ACCENT"),
        whiteSpace: "nowrap",
      }}
    >
      <Icon path={icon} size={14} color={c("ON_ACCENT")} />
      {label}
    </div>
  );
}

export function SectionHeading({ title, count }: { title: string; count: number }) {
  return (
    <div style={{ flex: "none", padding: "14px 4px 4px", display: "flex", alignItems: "center", gap: 8 }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: c("TEXT_MUTED") }}>{title}</div>
      <div style={{ ...mono, fontSize: 11, color: c("TEXT_FAINT") }}>{count}</div>
      <div style={{ flex: 1, height: 1, background: c("DIVIDER") }} />
    </div>
  );
}

export const panelStyle = {
  flex: "none",
  padding: 14,
  borderRadius: 10,
  background: c("SIDEBAR"),
  border: `1px solid ${c("DIVIDER")}`,
  overflowX: "auto",
} as const;
