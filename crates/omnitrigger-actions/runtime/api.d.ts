/** Node 动作宿主 API v1。受信任的本地代码，与当前用户具有相同权限。 */
export interface BrowserOptions {
  /** browser_instance_id；省略时只接受恰好一个已连接 profile。 */
  profile?: string;
  /** 默认 10000，范围 1–60000；同时受动作总运行时限约束。 */
  timeoutMs?: number;
}
/** 与原生 send_shortcut 配置共用字段、进程规范化和校验。 */
export interface ShortcutTiming {
  key_down_delay_ms?: number;
  process_overrides?: Record<string, number>;
}
export interface BrowserFault extends Error {
  code: string;
  method?: string;
  /** unknown 不能理解为操作失败或可以重试。 */
  effectState?: 'not_started' | 'unknown';
  requiredPermissions?: string[];
}
export interface DesktopFault extends Error {
  code: string;
  method?: 'target.info' | 'target.context' | 'keyboard.shortcut' | 'keyboard.sequence' | 'keyboard.text' | 'selection.copy' | 'clipboard.read' | 'clipboard.write' | 'window.action' | 'ui.choose';
  /** 仅描述本次 API 调用，不表示脚本之前的步骤没有副作用。 */
  effectState: 'not_started' | 'unknown';
}
export interface ChoiceItem {
  /** 必填且唯一的稳定 ID，最多 128 个 UTF-16 单元；不返回显示序号。 */
  id: string;
  /** 单行非空文本，最多 256 个 UTF-16 单元。 */
  label: string;
  /** 单行说明，最多 512 个 UTF-16 单元。 */
  description?: string;
}
export interface ChooseOptions {
  /** 1–200 项；参数总量最多 32 KiB UTF-8 JSON，不接受额外字段。 */
  items: readonly ChoiceItem[];
  /** 默认“选择一项”，最多 128 个 UTF-16 单元。 */
  title?: string;
  /** 默认“搜索”，最多 128 个 UTF-16 单元。搜索匹配 ID、标签和说明。 */
  placeholder?: string;
  /** 默认 30000，范围 1–60000 毫秒；另受动作总运行时限约束。 */
  timeoutMs?: number;
}
export type ChooseResult = {status: 'selected'; id: string}
  | {status: 'cancelled'; reason: 'dismissed' | 'timeout'};
export interface BrowserApi {
  profiles(options?: Pick<BrowserOptions, 'timeoutMs'>): Promise<Array<{
    browser_instance_id: string;
    session_id: string;
    name: string;
    [key: string]: unknown;
  }>>;
  /** 方法不带 chrome. 前缀；参数为 JSON 数组。undefined 与 null 保持区别。 */
  call<T = unknown>(method: string, args?: unknown[], options?: BrowserOptions): Promise<T>;
  /** 动态远程 Proxy，例如 await ctx.browser.chrome.bookmarks.getTree()。选项请使用 call。 */
  chrome: any;
}
export interface ActionContext {
  apiVersion: 1;
  input: Record<string, any>;
  invocation: Readonly<{invocation_id: string; binding_id: string}>;
  packageRoot: string;
  signal: AbortSignal;
  delay(milliseconds: number): Promise<void>;
  process: {
    run(program: string, args?: string[], options?: {cwd?: string; allowNonZero?: boolean}): Promise<{code: number; stdout: string; stderr: string}>;
    start(program: string, args?: string[], options?: {cwd?: string}): Promise<{pid: number}>;
  };
  browser: BrowserApi;
  ui: {
    /** 需要应用界面。↑↓/Enter 或点击选择，Esc/关闭取消；动作取消和宿主退出抛错。
     * 后续输入仍核验本次提交的原目标；系统拒绝恢复前台时输入返回 target_changed。
     * 已有窗口返回 ui_busy，不排队。务必 await，不可遗留选择 Promise。
     */
    choose(options: ChooseOptions): Promise<ChooseResult>;
  };
  desktop: {
    /** 本次提交时捕获的目标，不在调用时重新选择前台。进程名含 .exe。 */
    target(): Promise<{processName: string; windowRole: "unknown" | "application" | "desktop" | "taskbar"; title: string; processId: number; isForeground: boolean}>;
    /** CLI 可直接使用的原目标值快照；30 秒内提交，不重新选择前台。 */
    targetContext(): Promise<{window_handle: string; process_id: number; thread_id: number; process_created_file_time: string; expires_at_unix_ms: number}>;
    /** 明确 VK，修饰键在前；仅确认系统接纳输入，不确认业务动作完成。 */
    sendShortcut(keys: number[], options?: {keyDownDelayMs?: number; timing?: never} | {timing: ShortcutTiming; keyDownDelayMs?: never}): Promise<{submitted: true}>;
    /** 修饰键跨越整组普通键；延迟在修饰键就绪后、每个普通键按下后等待。 */
    sendKeySequence(modifiers: number[], keys: number[], options?: {keyDownDelayMs?: number}): Promise<{submitted: true}>;
    /** Unicode 直接输入，不使用剪贴板。最多 65,536 个 UTF-16 单元，无 NUL。 */
    sendText(text: string): Promise<{submitted: true}>;
    /** 向原目标发送 Ctrl+C 并读取新文本。复制结果保留在剪贴板。 */
    copySelectionText(): Promise<string>;
    /** 操作本次捕获且仍为前台的窗口；仅表示系统接纳请求，关闭可能弹出保存提示。 */
    windowAction(operation: 'toggle_maximize' | 'minimize' | 'close'): Promise<{submitted: true}>;
  };
  clipboard: {
    /** 只读 Unicode 文本；没有文本格式时抛错。 */
    readText(): Promise<string>;
    /** 替换全部格式，不自动恢复。excludeFromHistory 为临时回贴请求排除历史采集；仅遵守标记的采集器有效。 */
    writeText(text: string, options?: {excludeFromHistory?: boolean}): Promise<{written: true; sequenceNumber: number; writeId?: string}>;
  };
}
