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
type ScriptContext = Readonly<{
    apiVersion: 1;
    input: Record<string, any>;
    /** 动作包提供；直接使用 CLI 且未传元数据时为 null。 */
    invocation: Readonly<{invocation_id: string; binding_id: string}> | null;
    packageRoot: string | null;
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
