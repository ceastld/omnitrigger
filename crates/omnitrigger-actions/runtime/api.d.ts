/** Node 动作宿主 API v1。受信任的本地代码，与当前用户具有相同权限。 */
export interface BrowserOptions {
  /** browser_instance_id；省略时只接受恰好一个已连接 profile。 */
  profile?: string;
  /** 默认 10000，范围 1–60000；同时受动作总运行时限约束。 */
  timeoutMs?: number;
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
  method?: 'target.info' | 'keyboard.shortcut';
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
    target(): Promise<{processName: string; title: string; processId: number; isForeground: boolean}>;
    /** 明确 VK，修饰键在前；仅确认系统接纳输入，不确认业务动作完成。 */
    sendShortcut(keys: number[]): Promise<{submitted: true}>;
  };
}
