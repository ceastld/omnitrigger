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
  method?: 'target.info' | 'target.context' | 'keyboard.shortcut' | 'keyboard.sequence' | 'keyboard.text' | 'selection.copy' | 'clipboard.read' | 'clipboard.write';
  /** 仅描述本次 API 调用，不表示脚本之前的步骤没有副作用。 */
  effectState: 'not_started' | 'unknown';
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
  process: {
    run(program: string, args?: string[], options?: {cwd?: string; allowNonZero?: boolean}): Promise<{code: number; stdout: string; stderr: string}>;
    start(program: string, args?: string[], options?: {cwd?: string}): Promise<{pid: number}>;
  };
  browser: BrowserApi;
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
  };
  clipboard: {
    /** 只读 Unicode 文本；没有文本格式时抛错。 */
    readText(): Promise<string>;
    /** 替换全部格式，不自动恢复。excludeFromHistory 为临时回贴请求排除历史采集；仅遵守标记的采集器有效。 */
    writeText(text: string, options?: {excludeFromHistory?: boolean}): Promise<{written: true; sequenceNumber: number}>;
  };
}
