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
  method?: 'target.info' | 'target.context' | 'keyboard.shortcut' | 'keyboard.sequence' | 'keyboard.text' | 'selection.copy' | 'selection.copy_data' | 'clipboard.read' | 'clipboard.read_data' | 'clipboard.write' | 'window.action' | 'ui.choose' | 'ui.notify';
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
export interface NotifyOptions {
  /** 必填纯文本，非空白、有效 Unicode，最多 1024 个 UTF-16 单元；允许 CR/LF/Tab，其余控制字符拒绝。 */
  message: string;
  /** 默认 OmniTrigger；非空白单行有效 Unicode，最多 128 个 UTF-16 单元。 */
  title?: string;
  /** 默认 info。 */
  level?: 'info' | 'success' | 'warning' | 'error';
  /** 默认 topRight；相对于主显示器工作区。 */
  placement?: 'topRight' | 'bottomRight' | 'center';
  /** 默认 3000，整数 1000–30000 毫秒；返回不等待显示时长。 */
  durationMs?: number;
}
export type NotifyResult = {submitted: true};
/** 文本优先，包括空文本。sequenceNumber 为 u32，不证明数据生产者身份。
 * 文件路径仅是数据；不查询状态、不枚举或打开它们。
 */
export type ClipboardDataSnapshot = {kind: 'text'; text: string; sequenceNumber: number}
  | {kind: 'files'; paths: string[]; sequenceNumber: number};
export type SelectionDataSnapshot = ClipboardDataSnapshot & {changed: boolean};
export interface SelectionCopyOptions {
  /** 序号等待预算，默认 600，整数 1–5000；不是所有 Win32 调用的硬期限。 */
  timeoutMs?: number;
  /** 默认 15，整数 1–1000，且不得超过 timeoutMs；不自动调整默认值。 */
  pollIntervalMs?: number;
  /** 序号等待后可选稳定等待，默认 30，整数 0–1000 毫秒。 */
  settleMs?: number;
  /** 默认 false；true 允许读取序号未变化的当前数据，并返回 changed=false。 */
  allowUnchanged?: boolean;
}
/** 非原子元数据快照；后续打开必须重新检查。 */
export type LocalPathStat = {kind: 'file' | 'directory' | 'missing' | 'other'};
export interface FileStatFault extends Error {
  code: 'invalid_input' | 'path_not_local' | 'path_unavailable' | 'access_denied' | 'fs_error' | 'fs_unavailable' | 'fs_busy' | 'cancelled' | 'timeout';
  method: 'fs.stat';
  /** 仅此只读调用无副作用，不代表脚本之前的步骤没有副作用。 */
  effectState: 'not_started';
}
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
  fs: {
    /** 普通盘符绝对路径，1–32767 UTF-16 单元；只查固定本地物理盘元数据。
     * 不打开内容或枚举目录；拒绝链接、占位及云盘路径，无本机宿主为 fs_unavailable。
     * 缺失目标/祖先返回 missing；访问拒绝和其他错误抛错，不能当作 missing。
     * 词法预检不替代平台复验。单调用内并发返回 fs_busy，传输限时 5 秒且不重试。
     */
    stat(path: string): Promise<LocalPathStat>;
  };
  process: {
    run(program: string, args?: string[], options?: {cwd?: string; allowNonZero?: boolean}): Promise<{code: number; stdout: string; stderr: string}>;
    start(program: string, args?: string[], options?: {cwd?: string}): Promise<{pid: number}>;
  };
  browser: BrowserApi;
  ui: {
    /** 需要应用界面；仅确认已接纳并创建通知请求，不证明用户已读。
     * 不激活窗口，纯文本显示；已有通知返回 ui_busy，不排队或覆盖。
     * await 只等创建回执；脚本正常结束后通知按时长存活，应用退出收尾。
     */
    notify(options: NotifyOptions): Promise<NotifyResult>;
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
    /** 向本次捕获且仍为前台的原目标发送一次左 Ctrl+C，不重抓目标或抢焦点。
     * 返回最终文本/文件列表快照；changed 只比较最终序号与复制前序号，不证明生产者。
     * 默认未变化抛 selection_unchanged；不恢复剪贴板，取消不撤销已发送输入。
     * 客户端最多等待 8 秒；不能保证取消立即抢占阻塞 Win32 调用。
     */
    copySelectionData(options?: SelectionCopyOptions): Promise<SelectionDataSnapshot>;
    /** 操作本次捕获且仍为前台的窗口；仅表示系统接纳请求，关闭可能弹出保存提示。 */
    windowAction(operation: 'toggle_maximize' | 'minimize' | 'close'): Promise<{submitted: true}>;
  };
  clipboard: {
    /** 只读 Unicode 文本；没有文本格式时抛错。 */
    readText(): Promise<string>;
    /** 需要应用 native bridge；文本最多 65536 UTF-16 单元（可为空），无 NUL 或孤立代理项。
     * 文件最多 64 项，每项非空有效 Unicode、无 NUL，总计最多 65536 UTF-16 单元；[] 合法。
     * 路径不 trim、不验证绝对性/云盘/实际文件，也不查询或打开。
     * 无支持格式抛 unsupported_clipboard_format，不将图片/自定义格式当空文本。
     */
    readData(): Promise<ClipboardDataSnapshot>;
    /** 替换全部格式，不自动恢复。excludeFromHistory 为临时回贴请求排除历史采集；仅遵守标记的采集器有效。 */
    writeText(text: string, options?: {excludeFromHistory?: boolean}): Promise<{written: true; sequenceNumber: number; writeId?: string}>;
  };
}
