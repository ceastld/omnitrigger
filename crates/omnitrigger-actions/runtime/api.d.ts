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
}
