// `--export-trigger-preview` 的数据格式。行上的键位、鼠标部位、筛选维度与说明文字
// 都由应用代码算好；这里只做集合运算，不重新解释组合。

export type FacetKey = "input" | "action" | "scope" | "status";
export type GroupKey = "prefix" | "action" | "scope" | "input" | "none";

export interface Cap {
  label: string;
  mouse: boolean;
  prefix: boolean;
  arrow: string | null;
  /** 位置条件的完整描述，键帽只显示位置名称。 */
  title: string | null;
}

export interface Row {
  id: number;
  name: string;
  enabled: boolean;
  icon: string;
  tint: string;
  caps: Cap[];
  detail: string;
  detail_color: string;
  text: string;
  prefix: string;
  facets: Record<FacetKey, string>;
  key_terminal: boolean;
  key_slots: number[];
  held_slots: number[];
  mouse_terminal: string[];
  mouse_prefix: string[];
}

export interface KeySlot {
  vk: number;
  label: string;
  x: number;
  y: number;
  w: number;
}

export interface Preview {
  format: number;
  tokens: {
    dark: Record<string, string>;
    light: Record<string, string>;
    ui_font: string;
    mono_font: string;
  };
  icons: Record<string, string>;
  facets: { key: FacetKey; label: string; options: string[] }[];
  group_by: { key: GroupKey; label: string }[];
  keyboard: { width: number; height: number; slots: KeySlot[] };
  mouse_parts: string[];
  rows: Row[];
}

export type Filter = Partial<Record<FacetKey, string>>;

export const DISABLED = "已停用";

export function tokens(query: string): string[] {
  return query.split(/\s+/).filter(Boolean).map((word) => word.toLowerCase());
}

function matchesQuery(row: Row, words: string[]): boolean {
  return words.every((word) => row.text.includes(word));
}

function accepts(row: Row, filter: Filter, skip?: FacetKey): boolean {
  return (Object.keys(filter) as FacetKey[]).every(
    (facet) => facet === skip || filter[facet] === undefined || row.facets[facet] === filter[facet],
  );
}

export function visible(rows: Row[], words: string[], filter: Filter): Row[] {
  return rows.filter((row) => matchesQuery(row, words) && accepts(row, filter));
}

export function facetCount(rows: Row[], words: string[], filter: Filter, facet: FacetKey, value: string): number {
  return rows.filter((row) => matchesQuery(row, words) && accepts(row, filter, facet) && row.facets[facet] === value)
    .length;
}

export interface Group {
  key: string;
  label: string;
  rows: Row[];
  disabled: number;
}

export function browse(rows: Row[], words: string[], filter: Filter, groupBy: GroupKey): Group[] {
  const groups: Group[] = [];
  for (const row of visible(rows, words, filter)) {
    const [key, label] =
      groupBy === "prefix"
        ? row.prefix === ""
          ? ["", "无前缀"]
          : [row.prefix, row.prefix]
        : groupBy === "none"
          ? ["", ""]
          : [row.facets[groupBy], row.facets[groupBy]];
    let group = groups.find((candidate) => candidate.key === key);
    if (!group) {
      group = { key, label, rows: [], disabled: 0 };
      groups.push(group);
    }
    group.rows.push(row);
    if (row.facets.status === DISABLED) group.disabled += 1;
  }
  return groups;
}

/** 以键盘按键结尾的前缀层，按首次出现排列；空名称表示无前缀。 */
export function keyLayers(rows: Row[]): [string, number][] {
  const layers: [string, number][] = [];
  for (const row of rows.filter((candidate) => candidate.key_terminal)) {
    const layer = layers.find(([name]) => name === row.prefix);
    if (layer) layer[1] += 1;
    else layers.push([row.prefix, 1]);
  }
  return layers;
}

export interface KeyboardView {
  bound: Set<number>;
  boundDisabled: Set<number>;
  held: Set<number>;
  rows: Row[];
  offLayout: number;
}

export function keyboardView(rows: Row[], layer: string | null, picked: number | null): KeyboardView {
  const view: KeyboardView = { bound: new Set(), boundDisabled: new Set(), held: new Set(), rows: [], offLayout: 0 };
  const enabled = new Set<number>();
  for (const row of rows) {
    if (!row.key_terminal || (layer !== null && row.prefix !== layer)) continue;
    if (layer !== null && view.held.size === 0) row.held_slots.forEach((vk) => view.held.add(vk));
    if (row.key_slots.length === 0) view.offLayout += 1;
    for (const vk of row.key_slots) {
      view.bound.add(vk);
      if (row.facets.status !== DISABLED) enabled.add(vk);
    }
    if (picked === null || row.key_slots.includes(picked)) view.rows.push(row);
  }
  view.bound.forEach((vk) => {
    if (!enabled.has(vk)) view.boundDisabled.add(vk);
  });
  return view;
}

export interface PartCount {
  terminal: number;
  prefix: number;
}

export function partCount(rows: Row[], part: string): PartCount {
  let terminal = 0;
  let prefix = 0;
  for (const row of rows) {
    if (row.mouse_terminal.includes(part)) terminal += 1;
    else if (row.mouse_prefix.includes(part)) prefix += 1;
  }
  return { terminal, prefix };
}

export function mouseRows(rows: Row[], picked: string | null): Row[] {
  return rows.filter((row) =>
    picked === null
      ? row.mouse_terminal.length + row.mouse_prefix.length > 0
      : row.mouse_terminal.includes(picked) || row.mouse_prefix.includes(picked),
  );
}
