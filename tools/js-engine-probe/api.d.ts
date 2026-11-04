/** 同步实验 API。原生异常可在脚本中 catch，并通过 error.code 区分。 */
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
    clipboard: { readText(): string; writeText(text: string): void };
    /** 系统接纳请求，不等于网页或文件已经打开完成。 */
    open(target: string): void;
}>;
declare const ctx: ScriptContext;
