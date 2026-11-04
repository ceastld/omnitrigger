/** 入口返回的 JSON 数据；不包含 Promise、undefined、函数或平台对象。数值须为有限数。 */
type ScriptJsonValue = null | boolean | number | string | readonly ScriptJsonValue[] | {
    readonly [key: string]: ScriptJsonValue;
};
/** 标准同步入口。用 @type {ScriptMain} 标注 main，可同时检查上下文与返回值。 */
type ScriptMain = (ctx: ScriptContext) => ScriptJsonValue;

/** 同步宿主 API。原生异常可在脚本中 catch，并通过 error.code 区分。 */
interface ScriptDesktopFault extends Error {
    code: string;
    method?: string;
    /** 只描述本次 API；不能用于判断此前脚本步骤是否有副作用。 */
    effectState: 'not_started' | 'unknown';
}
interface ScriptShortcutTiming {
    key_down_delay_ms?: number;
    process_overrides?: Record<string, number>;
}
interface ScriptChoiceItem {
    /** 唯一稳定 ID，最多 128 个 UTF-16 单元。 */
    id: string;
    /** 单行非空文本，最多 256 个 UTF-16 单元。 */
    label: string;
    /** 单行说明，最多 512 个 UTF-16 单元。 */
    description?: string;
}
interface ScriptChooseOptions {
    /** 1–200 项；UTF-8 JSON 参数总量最多 32 KiB，不接受额外字段。 */
    items: readonly ScriptChoiceItem[];
    /** 默认“选择一项”，最多 128 个 UTF-16 单元。 */
    title?: string;
    /** 默认“搜索”，匹配 ID、标签和说明，最多 128 个 UTF-16 单元。 */
    placeholder?: string;
    /** 默认 30000，范围 1–60000 毫秒；另受动作总时限约束。 */
    timeoutMs?: number;
}
type ScriptChooseResult = {status: 'selected'; id: string}
    | {status: 'cancelled'; reason: 'dismissed' | 'timeout'};
interface ScriptBrowserOptions {
    /** browser_instance_id，省略时只接受恰好一个已连接 profile。 */
    profile?: string;
    /** 单次 RPC 的等待时限，默认 10000，范围 1–60000 毫秒；另受动作总时限约束。 */
    timeoutMs?: number;
}
interface ScriptBrowserFault extends ScriptDesktopFault {
    requiredPermissions?: string[];
}
type ScriptContext = Readonly<{
    apiVersion: 1;
    input: Record<string, any>;
    /** 动作包提供；直接使用 CLI 且未传元数据时为 null。 */
    invocation: Readonly<{invocation_id: string; binding_id: string}> | null;
    packageRoot: string | null;
    /** 同步等待 0–60000 毫秒的整数；受动作总时限约束。取消会终止独立宿主。 */
    delay(milliseconds: number): void;
    fs: {
        /** UTF-8 文件，绝对路径，最多 64 KiB。 */
        readText(path: string): string;
        /** 默认不覆盖；不自动创建目录。失败可能留下部分文件。 */
        writeText(path: string, text: string, options?: { overwrite?: boolean }): void;
    };
    process: {
        /** 等待直接子进程。标准输出和标准错误不采集。非零退出由脚本判断。 */
        run(program: string, args?: string[], options?: { timeoutMs?: number }): {
            pid: number; exitCode: number | null; success: boolean;
        };
        /** 创建独立程序后返回，不随脚本结束关闭。 */
        start(program: string, args?: string[]): { pid: number };
    };
    ui: {
        /** 需要应用 GPUI 界面。同步等待发生在动作线程，Hook 不等待。
         * ↑↓/Enter 或点击选择；Esc/关闭取消，动作取消与宿主退出抛错。
         * 已有窗口返回 ui_busy。后续输入保持原目标身份；失焦返回 target_changed。
         */
        choose(options: ScriptChooseOptions): ScriptChooseResult;
    };
    desktop: {
        /** 本次提交时捕获的目标，不在调用时重新选择前台；独立 CLI 不提供目标。 */
        target(): {processName: string; windowRole: 'unknown' | 'application' | 'desktop' | 'taskbar'; title: string; processId: number; isForeground: boolean};
        /** 30 秒内可提交的原目标快照，句柄和 FILETIME 保持字符串精度。 */
        targetContext(): {window_handle: string; process_id: number; thread_id: number; process_created_file_time: string; expires_at_unix_ms: number};
        /** VK 整数，修饰键在前。仅确认系统接纳输入。 */
        sendShortcut(keys: number[], options?: {keyDownDelayMs?: number; timing?: never} | {timing: ScriptShortcutTiming; keyDownDelayMs?: never}): {submitted: true};
        sendKeySequence(modifiers: number[], keys: number[], options?: {keyDownDelayMs?: number}): {submitted: true};
        /** 有效 Unicode，无 NUL，最多 65,536 个 UTF-16 单元，不经过剪贴板。 */
        sendText(text: string): {submitted: true};
        /** 向原目标发送 Ctrl+C 并读取新文本，复制结果保留在剪贴板。 */
        copySelectionText(): string;
        /** 操作本次捕获且仍为前台的窗口；仅表示系统接纳请求，关闭可能弹出保存提示。 */
        windowAction(operation: 'toggle_maximize' | 'minimize' | 'close'): {submitted: true};
    };
    browser: {
        profiles(options?: Pick<ScriptBrowserOptions, 'timeoutMs'>): Array<{browser_instance_id: string; session_id: string; name: string; [key: string]: unknown}>;
        /** 方法不带 chrome. 前缀；同步返回，undefined 与 null 保持区别。 */
        call<T = unknown>(method: string, args?: unknown[], options?: ScriptBrowserOptions): T;
        /** 同步远程 Proxy，例如 ctx.browser.chrome.bookmarks.getTree()。选项使用 call。 */
        chrome: any;
    };
    clipboard: {
        readText(): string;
        /** 替换全部格式。独立 CLI 不支持选项并返回 void；应用接入返回写入回执。 */
        writeText(text: string, options?: {excludeFromHistory?: boolean}): void | {written: true; sequenceNumber: number; writeId?: string};
    };
    /** 系统接纳请求，不等于网页或文件已经打开完成。 */
    open(target: string): void;
}>;
declare const ctx: ScriptContext;
