// 触发器页中栏：对应 main.rs 的 render_list / render_trigger_toolbar / render_trigger_row / render_group_header。
import { useEffect, useRef, useState } from "react";
import {
  browse,
  facetCount,
  tokens,
  visible,
  DISABLED,
  type FacetKey,
  type Filter,
  type GroupKey,
  type Row,
} from "./data";
import { Caption, Combo, Heading, Icon, IconTile, PrimaryButton, Segment, Segments, Switch, c, mono, usePreview } from "./ui";
import { KeyboardView } from "./KeyboardView";
import { MouseView } from "./MouseView";

export type View = "list" | "keyboard" | "mouse";
type Menu = FacetKey | "group" | null;

export function TriggerPage({ initialView = "list" }: { initialView?: View }) {
  const preview = usePreview();
  const [rows, setRows] = useState(preview.rows);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>({});
  const [groupBy, setGroupBy] = useState<GroupKey>("prefix");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [menu, setMenu] = useState<Menu>(null);
  const [view, setView] = useState<View>(initialView);
  const [selected, setSelected] = useState<number | null>(preview.rows[0]?.id ?? null);

  const words = tokens(query);
  const groups = browse(rows, words, filter, groupBy);
  const shown = groups.reduce((sum, group) => sum + group.rows.length, 0);
  const shownRows = visible(rows, words, filter);
  const grouped = groupBy !== "none";
  const toggle = (id: number) =>
    setRows((current) =>
      current.map((row) =>
        row.id === id
          ? {
              ...row,
              enabled: !row.enabled,
              facets: { ...row.facets, status: row.enabled ? DISABLED : "已启用" },
            }
          : row,
      ),
    );
  // 左侧分组导航：点击跳到该组，滚动时高亮当前所在的组。
  const list = useRef<HTMLDivElement>(null);
  const [current, setCurrent] = useState<string | null>(null);
  // 点击导航后的平滑滚动期间保持所点的组高亮，不被滚动位置覆盖。
  const jumping = useRef(0);
  const showNav = view === "list" && grouped && groups.length > 1;
  const spy = () => {
    const container = list.current;
    if (!container || Date.now() < jumping.current) return;
    const top = container.getBoundingClientRect().top;
    let active: string | null = null;
    container.querySelectorAll<HTMLElement>("[data-group]").forEach((header) => {
      if (header.getBoundingClientRect().top - top <= 8) active = header.dataset.group ?? null;
    });
    // 滚到底时最后几组可能永远到不了顶端，此时取最后一组。
    if (container.scrollTop + container.clientHeight >= container.scrollHeight - 2) {
      const headers = container.querySelectorAll<HTMLElement>("[data-group]");
      active = headers[headers.length - 1]?.dataset.group ?? active;
    }
    setCurrent(active ?? container.querySelector<HTMLElement>("[data-group]")?.dataset.group ?? null);
  };
  useEffect(spy, [groupBy, query, filter, collapsed, view]);
  const jump = (key: string) => {
    setCurrent(key);
    jumping.current = Date.now() + 700;
    setCollapsed((set) => {
      if (!set.has(key)) return set;
      const next = new Set(set);
      next.delete(key);
      return next;
    });
    requestAnimationFrame(() => {
      const container = list.current;
      const header = container?.querySelector<HTMLElement>(`[data-group="${CSS.escape(key)}"]`);
      if (container && header) {
        container.scrollTo({
          top: header.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop,
          behavior: "smooth",
        });
      }
    });
  };

  const renderRow = (row: Row) => (
    <TriggerRow key={row.id} row={row} selected={selected === row.id} onSelect={() => setSelected(row.id)} onToggle={() => toggle(row.id)} />
  );

  return (
    <div
      onMouseDown={() => setMenu(null)}
      style={{
        flex: 1,
        minWidth: 0,
        height: "100%",
        boxSizing: "border-box",
        padding: "6px 18px 16px 20px",
        display: "flex",
        flexDirection: "column",
        gap: 14,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 12 }}>
        <Heading title="触发器" subtitle="按住前缀再按目标键或滚轮，支持应用范围" />
        <PrimaryButton icon="icons/record.svg" label="录制组合" />
      </div>
      <Toolbar
        rows={rows}
        words={words}
        query={query}
        setQuery={setQuery}
        filter={filter}
        setFilter={setFilter}
        groupBy={groupBy}
        setGroupBy={setGroupBy}
        menu={menu}
        setMenu={setMenu}
        view={view}
        setView={setView}
        shown={shown}
      />
      <div style={{ flex: 1, minHeight: 0, display: "flex", gap: 14 }}>
        {showNav && (
          <GroupNav
            items={nestNav(
              groups.map((group) => ({
                key: `${groupBy}:${group.key}`,
                label: group.label,
                count: group.rows.length,
                disabled: group.disabled,
                path: groupBy === "prefix" && group.key !== "" ? group.rows[0].caps.filter((cap) => cap.prefix).map((cap) => cap.label) : [],
              })),
            )}
            current={current}
            onPick={jump}
          />
        )}
      <div ref={list} onScroll={spy} style={{ flex: 1, minWidth: 0, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 2 }}>
        {view === "list" &&
          groups.map((group, index) => {
            const key = `${groupBy}:${group.key}`;
            const isCollapsed = grouped && collapsed.has(key);
            return (
              <div key={key} style={{ display: "contents" }}>
                {grouped && (
                  <div data-group={key} style={{ flex: "none" }}>
                  <GroupHeader
                    index={index}
                    label={group.label}
                    prefixCaps={groupBy === "prefix" && group.key !== "" ? group.rows[0].caps.filter((cap) => cap.prefix) : null}
                    count={group.rows.length}
                    disabled={group.disabled}
                    collapsed={isCollapsed}
                    onClick={() =>
                      setCollapsed((current) => {
                        const next = new Set(current);
                        if (!next.delete(key)) next.add(key);
                        return next;
                      })
                    }
                  />
                  </div>
                )}
                {!isCollapsed && group.rows.map(renderRow)}
              </div>
            );
          })}
        {view === "keyboard" && shown > 0 && <KeyboardView rows={shownRows} renderRow={renderRow} />}
        {view === "mouse" && shown > 0 && <MouseView rows={shownRows} renderRow={renderRow} />}
        {rows.length > 0 && shown === 0 && (
          <div style={{ padding: "56px 0", display: "flex", flexDirection: "column", alignItems: "center", gap: 12 }}>
            <IconTile path="icons/search.svg" color="ACCENT_2" size={48} />
            <div style={{ fontSize: 15, fontWeight: 600, color: c("TEXT_SOFT") }}>没有符合条件的触发器</div>
          </div>
        )}
      </div>
      </div>
      <Caption>最多 512 条触发器，每条最多 8 个前缀。单独按下未用于组合的前缀时，原输入会补发给应用。</Caption>
    </div>
  );
}

interface ToolbarProps {
  rows: Row[];
  words: string[];
  query: string;
  setQuery: (query: string) => void;
  filter: Filter;
  setFilter: (filter: Filter) => void;
  groupBy: GroupKey;
  setGroupBy: (group: GroupKey) => void;
  menu: Menu;
  setMenu: (menu: Menu) => void;
  view: View;
  setView: (view: View) => void;
  shown: number;
}

function Toolbar(props: ToolbarProps) {
  const preview = usePreview();
  const { rows, words, filter, setFilter, menu, setMenu } = props;
  const enabled = rows.filter((row) => row.facets.status !== DISABLED).length;
  const [focused, setFocused] = useState(false);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <label
          style={{
            flex: 1,
            minWidth: 0,
            height: 34,
            padding: "0 12px",
            boxSizing: "border-box",
            borderRadius: 6,
            background: c("BACKGROUND"),
            border: `1px solid ${focused ? c("ACCENT", 0.7) : c("BORDER_MID")}`,
            display: "flex",
            alignItems: "center",
            gap: 8,
          }}
        >
          <Icon path="icons/search.svg" size={14} color={focused ? c("ACCENT") : c("TEXT_FAINT")} />
          <input
            value={props.query}
            onChange={(event) => props.setQuery(event.target.value)}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            placeholder="搜索名称、动作、组合或应用"
            style={{ flex: 1, minWidth: 0, border: "none", outline: "none", background: "transparent", color: c("TEXT"), fontSize: 13, font: "inherit" }}
          />
        </label>
        <Segments>
          <Segment label="列表" active={props.view === "list"} onClick={() => props.setView("list")} />
          <Segment label="键盘" active={props.view === "keyboard"} onClick={() => props.setView("keyboard")} />
          <Segment label="鼠标" active={props.view === "mouse"} onClick={() => props.setView("mouse")} />
        </Segments>
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 6 }}>
        {preview.facets.map((facet) => {
          const current = filter[facet.key];
          const options: Option[] = [{ label: "全部", count: null, selected: current === undefined, pick: () => setFilter({ ...filter, [facet.key]: undefined }) }];
          for (const value of facet.options) {
            const count = facetCount(rows, words, filter, facet.key, value);
            if (count > 0 || current === value) {
              options.push({ label: value, count, selected: current === value, pick: () => setFilter({ ...filter, [facet.key]: value }) });
            }
          }
          return (
            <Dropdown
              key={facet.key}
              text={`${facet.label}：${current ?? "全部"}`}
              active={current !== undefined}
              open={menu === facet.key}
              onToggle={() => setMenu(menu === facet.key ? null : facet.key)}
              onPick={() => setMenu(null)}
              options={options}
            />
          );
        })}
        {(words.length > 0 || Object.values(filter).some((value) => value !== undefined)) && (
          <div
            className="hover-text"
            onClick={() => {
              props.setQuery("");
              setFilter({});
            }}
            style={{ padding: "0 6px", fontSize: 12, color: c("TEXT_DIM"), cursor: "pointer" }}
          >
            清除筛选
          </div>
        )}
        <div style={{ flex: 1 }} />
        <div style={{ ...mono, flex: "none", fontSize: 11.5, color: c("TEXT_FAINT") }}>
          {props.shown} / {rows.length} 条 · {enabled} 启用 · 本次 0
        </div>
        {props.view === "list" && (
          <Dropdown
            text={preview.group_by.find((group) => group.key === props.groupBy)?.label ?? ""}
            active={false}
            alignRight
            open={menu === "group"}
            onToggle={() => setMenu(menu === "group" ? null : "group")}
            onPick={() => setMenu(null)}
            options={preview.group_by.map((group) => ({
              label: group.label,
              count: null,
              selected: group.key === props.groupBy,
              pick: () => props.setGroupBy(group.key),
            }))}
          />
        )}
      </div>
    </div>
  );
}

interface Option {
  label: string;
  count: number | null;
  selected: boolean;
  pick: () => void;
}

function Dropdown(props: {
  text: string;
  active: boolean;
  open: boolean;
  alignRight?: boolean;
  onToggle: () => void;
  onPick: () => void;
  options: Option[];
}) {
  return (
    <div style={{ position: "relative", flex: "none" }} onMouseDown={(event) => event.stopPropagation()}>
      <div
        className="hover-border"
        onClick={props.onToggle}
        style={{
          height: 28,
          padding: "0 10px",
          boxSizing: "border-box",
          borderRadius: 14,
          border: `1px solid ${props.active || props.open ? c("ACCENT_LINE") : c("BORDER_MID")}`,
          background: props.active ? c("ACCENT_BG") : undefined,
          display: "flex",
          alignItems: "center",
          gap: 6,
          fontSize: 12,
          color: props.active ? c("ACCENT") : c("TEXT_MUTED"),
          whiteSpace: "nowrap",
          cursor: "pointer",
        }}
      >
        {props.text}
        <span style={{ fontSize: 9, color: c("TEXT_FAINT") }}>{props.open ? "▲" : "▼"}</span>
      </div>
      {props.open && (
        <div
          style={{
            position: "absolute",
            top: 32,
            [props.alignRight ? "right" : "left"]: 0,
            zIndex: 10,
            minWidth: 176,
            padding: 4,
            borderRadius: 8,
            border: `1px solid ${c("BORDER_MID")}`,
            background: c("SURFACE"),
            boxShadow: "0 12px 32px -6px rgba(0,0,0,0.45)",
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          {props.options.map((option) => (
            <div
              key={option.label}
              className="hover-option"
              onClick={() => {
                option.pick();
                props.onPick();
              }}
              style={{
                height: 28,
                padding: "0 8px",
                borderRadius: 5,
                display: "flex",
                alignItems: "center",
                gap: 8,
                fontSize: 12,
                cursor: "pointer",
                ...(option.selected ? { background: c("SURFACE_HI"), color: c("ACCENT") } : {}),
              }}
            >
              <div style={{ flex: 1, whiteSpace: "nowrap" }}>{option.label}</div>
              {option.count !== null && <div style={{ ...mono, fontSize: 11, color: c("TEXT_FAINT") }}>{option.count}</div>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function TriggerRow({ row, selected, onSelect, onToggle }: { row: Row; selected: boolean; onSelect: () => void; onToggle: () => void }) {
  return (
    <div
      className={selected ? undefined : "hover-row"}
      style={{
        flex: "none",
        position: "relative",
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "10px 12px",
        borderRadius: 6,
        border: "1px solid transparent",
        background: selected ? c("SELECTED") : undefined,
      }}
    >
      {selected && (
        <div style={{ position: "absolute", left: -1, top: 18, bottom: 18, width: 2, borderRadius: 2, background: c("ACCENT") }} />
      )}
      <div
        onClick={onSelect}
        style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 10, cursor: "pointer", opacity: row.enabled ? 1 : 0.45 }}
      >
        <IconTile path={row.icon} color={row.tint} size={28} />
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 5 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <div className="truncate" style={{ flex: 1, minWidth: 0, fontSize: 13, fontWeight: 600 }}>
              {row.name}
            </div>
            {!row.enabled && <div style={{ flex: "none", fontSize: 11, color: c("TEXT_FAINT") }}>已停用</div>}
          </div>
          <div style={{ minWidth: 0, display: "flex", alignItems: "center", gap: 10 }}>
            <Combo caps={row.caps} />
            <div className="truncate" style={{ flex: 1, minWidth: 0, fontSize: 12, color: c(row.detail_color) }}>
              {row.detail}
            </div>
          </div>
        </div>
      </div>
      <Switch on={row.enabled} onClick={onToggle} />
    </div>
  );
}

function GroupHeader(props: {
  index: number;
  label: string;
  prefixCaps: Row["caps"] | null;
  count: number;
  disabled: number;
  collapsed: boolean;
  onClick: () => void;
}) {
  return (
    <div
      onClick={props.onClick}
      style={{
        flex: "none",
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: `${props.index === 0 ? 2 : 12}px 4px 4px`,
        cursor: "pointer",
      }}
    >
      <Icon path={props.collapsed ? "icons/arrow-right.svg" : "icons/arrow-down.svg"} size={10} color={c("TEXT_FAINT")} />
      {props.prefixCaps ? (
        <Combo caps={props.prefixCaps} />
      ) : (
        <div style={{ fontSize: 12, fontWeight: 600, color: c("TEXT_MUTED") }}>{props.label}</div>
      )}
      <div style={{ ...mono, fontSize: 11, color: c("TEXT_FAINT") }}>
        {props.disabled > 0 ? `${props.count} · ${props.disabled} 停用` : props.count}
      </div>
      <div style={{ flex: 1, height: 1, background: c("DIVIDER") }} />
    </div>
  );
}

interface NavItem {
  key: string;
  label: string;
  count: number;
  disabled: number;
  /** 按前缀分组时的前缀键帽序列，用来判断层级。 */
  path: string[];
}

interface NestedItem extends NavItem {
  depth: number;
  /** 子层沿用上层前缀的部分，淡色显示，例如 Space 下的 `Space + `。 */
  lead: string;
  rest: string;
}

/** 前缀是另一组前缀延长的组挂到那一组下面，按树的顺序排列。 */
function nestNav(items: NavItem[]): NestedItem[] {
  const startsWith = (path: string[], head: string[]) =>
    head.length > 0 && head.length < path.length && head.every((label, index) => path[index] === label);
  const parent = new Map<string, NavItem>();
  for (const item of items) {
    const candidates = items.filter((other) => startsWith(item.path, other.path));
    const nearest = candidates.sort((a, b) => b.path.length - a.path.length)[0];
    if (nearest) parent.set(item.key, nearest);
  }
  const out: NestedItem[] = [];
  const visit = (item: NavItem, depth: number) => {
    const above = parent.get(item.key);
    const lead = above ? `${above.path.join(" + ")} + ` : "";
    const rest = above ? item.path.slice(above.path.length).join(" + ") : item.label;
    out.push({ ...item, depth, lead, rest });
    for (const child of items.filter((other) => parent.get(other.key) === item)) visit(child, depth + 1);
  };
  for (const item of items.filter((candidate) => !parent.has(candidate.key))) visit(item, 0);
  return out;
}

/** 分组导航列：样式沿用应用侧栏的导航项，计数用等宽字体，整组停用时变暗。 */
function GroupNav({ items, current, onPick }: { items: NestedItem[]; current: string | null; onPick: (key: string) => void }) {
  return (
    <div
      style={{
        flex: "none",
        width: 168,
        minHeight: 0,
        overflowY: "auto",
        display: "flex",
        flexDirection: "column",
        gap: 2,
        paddingRight: 10,
        borderRight: `1px solid ${c("DIVIDER")}`,
      }}
    >
      <div style={{ padding: "2px 10px 6px", fontSize: 11, color: c("TEXT_FAINT") }}>分组</div>
      {items.map((item) => {
        const active = item.key === current;
        const allDisabled = item.disabled === item.count;
        return (
          <div
            key={item.key}
            className={active ? undefined : "hover-nav"}
            onClick={() => onPick(item.key)}
            title={item.label}
            style={{
              flex: "none",
              position: "relative",
              height: 30,
              padding: `0 10px 0 ${10 + item.depth * 14}px`,
              borderRadius: 6,
              display: "flex",
              alignItems: "center",
              gap: 8,
              fontSize: 12.5,
              cursor: "pointer",
              color: active ? c("TEXT") : allDisabled ? c("TEXT_FAINT") : c("TEXT_DIM"),
              background: active ? c("SURFACE_HI") : undefined,
              fontWeight: active ? 600 : undefined,
            }}
          >
            {active && <div style={{ position: "absolute", left: 0, top: 8, bottom: 8, width: 2, borderRadius: 2, background: c("ACCENT") }} />}
            <div className="truncate" style={{ flex: 1, minWidth: 0 }}>
              {item.lead && <span style={{ color: c("TEXT_FAINT"), fontWeight: 400 }}>{item.lead}</span>}
              {item.rest}
            </div>
            <div style={{ ...mono, flex: "none", fontSize: 11, color: c("TEXT_FAINT") }}>{item.count}</div>
          </div>
        );
      })}
    </div>
  );
}
