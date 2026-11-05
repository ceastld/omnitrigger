interface ScriptBrightnessStepOptions {
  direction:'up'|'down';
  strategy:'physical'|'wmi_first'|'physical_then_wmi';
}
type ScriptBrightnessStepCode='initialization_failed'|'monitor_unavailable'|'ambiguous_monitor'
  |'brightness_unavailable'|'brightness_failed'|'brightness_unknown';
type ScriptBrightnessStepNoWriteCode=Exclude<ScriptBrightnessStepCode,'brightness_unknown'>;
/** index是本次集合序号，数值范围仍由运行时校验。 */
type ScriptBrightnessStepReadRejected={index:number;status:'read_rejected';previousRaw:null;requestedRaw:null;currentRaw:null;errorCode:ScriptBrightnessStepCode};
type ScriptBrightnessStepUnchanged={index:number;status:'unchanged';previousRaw:number;requestedRaw:number;currentRaw:number;errorCode:null};
type ScriptBrightnessStepNoWriteMember=ScriptBrightnessStepReadRejected|ScriptBrightnessStepUnchanged;
type ScriptBrightnessStepMember=ScriptBrightnessStepNoWriteMember
  |{index:number;status:'set_unconfirmed'|'readback_unknown';previousRaw:number;requestedRaw:number;currentRaw:null;errorCode:ScriptBrightnessStepCode}
  |{index:number;status:'readback_different'|'confirmed';previousRaw:number;requestedRaw:number;currentRaw:number;errorCode:null};
type ScriptBrightnessStepPhysical={members:ScriptBrightnessStepMember[];errorCode:null}
  |{members:[];errorCode:ScriptBrightnessStepCode};
type ScriptBrightnessStepNoWritePhysical={members:ScriptBrightnessStepNoWriteMember[];errorCode:null}
  |{members:[];errorCode:ScriptBrightnessStepNoWriteCode};
type ScriptBrightnessStepWmiFailure={status:'failed';previousPercent:null;requestedPercent:null;currentPercent:null;errorCode:ScriptBrightnessStepCode};
type ScriptBrightnessStepWmiNoWrite={status:'zero_skipped'|'no_eligible_level';previousPercent:number;requestedPercent:null;currentPercent:number;errorCode:null}
  |(Omit<ScriptBrightnessStepWmiFailure,'errorCode'>&{errorCode:ScriptBrightnessStepNoWriteCode});
type ScriptBrightnessStepWmi=ScriptBrightnessStepWmiNoWrite|ScriptBrightnessStepWmiFailure
  |{status:'set';previousPercent:number;requestedPercent:number;currentPercent:number;errorCode:null};
type ScriptBrightnessStepStrategies<P,W>=
  |{strategy:'physical';physical:P;wmi:null}
  |{strategy:'wmi_first';physical:null;wmi:W}
  |{strategy:'physical_then_wmi';physical:P;wmi:W|null};
/** unknown为保守副作用状态；正常写入仍须检查逐项读回，取消不撤销设备变化。 */
type ScriptBrightnessStepResult=
  |({effectState:'unknown'}&ScriptBrightnessStepStrategies<ScriptBrightnessStepPhysical,ScriptBrightnessStepWmi>)
  |({effectState:'not_started'}&ScriptBrightnessStepStrategies<ScriptBrightnessStepNoWritePhysical,ScriptBrightnessStepWmiNoWrite>);

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
interface ScriptNotifyOptions {
    /** 必填纯文本，非空白有效 Unicode，最多 1024 个 UTF-16 单元；允许 CR/LF/Tab，其余控制字符拒绝。 */
    message: string;
    /** 默认 OmniTrigger；非空白单行有效 Unicode，最多 128 个 UTF-16 单元。 */
    title?: string;
    /** 默认 info。 */
    level?: 'info' | 'success' | 'warning' | 'error';
    /** 默认 topRight；相对于主显示器工作区。 */
    placement?: 'topRight' | 'bottomRight' | 'center';
    /** 默认 3000，整数 1000–30000 毫秒；不等待显示结束。 */
    durationMs?: number;
}
type ScriptNotifyResult = {submitted: true};
/** readData()/auto 文本优先（含空文本）；文件路径仅为数据，不查询或打开。序号为 u32。 */
type ScriptClipboardDataSnapshot = {kind: 'text'; text: string; sequenceNumber: number}
    | {kind: 'files'; paths: string[]; sequenceNumber: number};
type ScriptSelectionDataSnapshot = ScriptClipboardDataSnapshot & {changed: boolean};
interface ScriptSelectionCopyOptions {
    /** 默认 auto 文本优先（含空文本）；text/files 只读取指定格式，缺失或无效均不回退。 */
    format?: 'auto' | 'text' | 'files';
    /** 默认 600，整数 1–5000；仅为序号等待预算，不是全部 Win32 调用的硬期限。 */
    timeoutMs?: number;
    /** 默认 15，整数 1–1000 且 <= timeoutMs；不自动调整。 */
    pollIntervalMs?: number;
    /** 默认 30，整数 0–1000 毫秒。 */
    settleMs?: number;
    /** 默认 false；true 允许当前未变化数据，返回 changed=false。 */
    allowUnchanged?: boolean;
}
/** 非原子元数据快照；后续打开必须重新检查。 */
type ScriptLocalPathStat = {kind: 'file' | 'directory' | 'missing' | 'other'};
interface ScriptFileStatFault extends Error {
    code: 'invalid_input' | 'path_not_local' | 'path_unavailable' | 'access_denied' | 'fs_error' | 'fs_unavailable' | 'fs_busy' | 'cancelled' | 'timeout';
    method: 'fs.stat';
    /** 仅此只读调用无副作用，不涵盖此前脚本步骤。 */
    effectState: 'not_started';
}
interface ScriptBrowserOptions {
    /** browser_instance_id，省略时只接受恰好一个已连接 profile。 */
    profile?: string;
    /** 单次 RPC 的等待时限，默认 10000，范围 1–60000 毫秒；另受动作总时限约束。 */
    timeoutMs?: number;
}
interface ScriptBrowserFault extends ScriptDesktopFault {
    requiredPermissions?: string[];
}
/** u64 的规范十进制字符串；ID 非零，revision 可以为 "0"，不转换为 JS number。 */
type ScriptHistoryDecimal = string;
/** createdAt 为应用持久化的时间字符串；SDK 不重新解释时区或重写格式。 */
type ScriptClipboardHistoryItem = {
  id: ScriptHistoryDecimal;
  /** 单行摘要，最多 64 UTF-16 单元，不切断代理对；不是全文。 */
  preview: string;
  utf16Units: number;
  createdAt: string;
};
type ScriptClipboardHistoryList = {items: ScriptClipboardHistoryItem[]; revision: ScriptHistoryDecimal};
type ScriptClipboardHistoryText = {id: ScriptHistoryDecimal; text: string; createdAt: string; revision: ScriptHistoryDecimal};
type ScriptClipboardHistoryAppend = {id: ScriptHistoryDecimal; revision: ScriptHistoryDecimal; inserted: boolean};
type ScriptClipboardHistoryRemove = {removed: boolean; revision: ScriptHistoryDecimal};
type ScriptClipboardHistoryMethod = 'history.list' | 'history.get' | 'history.append' | 'history.remove';
interface ScriptClipboardHistoryFault extends Error {
  code: string;
  method: ScriptClipboardHistoryMethod;
  /** list/get 总为 not_started；写入发出后无法确认的失败为 unknown。 */
  effectState: 'not_started' | 'unknown';
}
interface ScriptClipboardHistoryApi {
  /** 默认 limit=100，整数 1..200；只返回摘要，不返回正文。 */
  list(options?: {limit?: number}): ScriptClipboardHistoryList;
  /** 不存在抛 history_item_missing；正文只供脚本内部使用，不应放进动作终态。 */
  get(options: {id: ScriptHistoryDecimal}): ScriptClipboardHistoryText;
  /** 显式保存文本：可为空、保留空白、有效 Unicode、无 NUL，最多 65536 UTF-16。
   * 连续相同文本可去重并 inserted=false；不自动重试，不建立监听或读系统剪贴板。
   */
  append(options: {text: string}): ScriptClipboardHistoryAppend;
  /** 不存在返回 removed=false；无变化不递增 revision。 */
  remove(options: {id: ScriptHistoryDecimal}): ScriptClipboardHistoryRemove;
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
        /** 同步只查普通盘符绝对路径的本地固定物理盘元数据，最多 32767 UTF-16 单元。
         * 不打开内容或枚举目录；平台独立拒绝链接、占位和云盘，词法预检不代替复验。
         * 缺失目标/祖先为 missing，访问拒绝及其他错误抛错；无本机宿主为 fs_unavailable。
         * 5 秒短时 RPC，不重试；元数据非原子，后续打开须重新检查。
         */
        stat(path: string): ScriptLocalPathStat;
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
        /** Windows 10+ 有界 UTF-8 输出采集；保留 run 原语义。
         * program 为明确本地绝对 .exe 路径，脚本显式传解释器；不搜索 PATH、不隐式 shell。
         * args 最多128项，无NUL，MSVCRT命令行最多32767 UTF-16（含终止）。
         * timeoutMs 为1..60000，默认10000；cwd 可选本地绝对目录。不同于所有OS调用的硬期限。
         * stdout/stderr 合计最多65536字节、严格UTF-8；溢出/坏编码/超时无部分回执。
         * 默认非零退出抛 process_exit_nonzero；allowNonZero=true 才返回非零code及完整输出。
         * 私有Job在创建时关联，成功/失败/宿主关闭时收尾Job内后代；不是任意外部服务进程树沙盒。
         * 取消由外层结束宿主；不保证阻塞OS调用中即时协作取消，不自动重试。
         * 输出供脚本内部处理，默认终态/错误不应回显正文。
         */
        capture(program: string, args?: string[], options?: { timeoutMs?: number; allowNonZero?: boolean; cwd?: string }): {
            code: number; stdout: string; stderr: string;
        };
        /** 创建独立程序后返回，不随脚本结束关闭。 */
        start(program: string, args?: string[]): { pid: number };
    };
    ui: {
        /** 只确认创建独立历史窗；动作结束后仍存活，关闭仅停止该窗自己的采集。无UI为ui_unavailable。 */
        openHistory(): {opened: true};
        /** 与Node入口共用本次Host的原生切换：无窗创建，已有窗等自身采集停止并移除后返回closed。
         * chooser/未完成关闭为ui_busy；关闭不需新目标；无UI为ui_unavailable。
         * 开始后取消/超时为unknown，不自动重试，不停止主窗采集。 */
        toggleHistory(): {opened: true} | {closed: true};
        /** 需要应用界面；同步等待创建回执，不等待显示时长，不激活窗口。
         * 返回仅证明接纳并创建通知请求，不证明用户已读；已有通知返回 ui_busy。
         * 脚本正常完成后按时长存活，应用退出收尾。仅纯文本，不执行 HTML 或链接。
         */
        notify(options: ScriptNotifyOptions): ScriptNotifyResult;
        /** 需要应用 GPUI 界面。同步等待发生在动作线程，Hook 不等待。
         * ↑↓/Enter 或点击选择；Esc/关闭取消，动作取消与宿主退出抛错。
         * 已有窗口返回 ui_busy。后续输入保持原目标身份；失焦返回 target_changed。
         */
        choose(options: ScriptChooseOptions): ScriptChooseResult;
    };
    desktop: {
        /** 同步，参数与Node入口共用严格工厂；逐项检查回执。 */
        stepBrightness(options: ScriptBrightnessStepOptions): ScriptBrightnessStepResult;
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
        /** 同步向捕获且仍为前台的原目标发送一次左 Ctrl+C，不重抓或抢焦点。
         * changed 比较最终序号与复制前序号，不证明生产者。默认未变抛 selection_unchanged。
         * 不恢复剪贴板，取消不撤销输入；客户端最多等待 8 秒，不保证抢占阻塞 Win32 调用。
         */
        copySelectionData(options?: ScriptSelectionCopyOptions): ScriptSelectionDataSnapshot;
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
        history: ScriptClipboardHistoryApi;
        readText(): string;
        /** 同步读应用 native bridge；无 bridge 抛 native_bridge_unavailable，不退回旧 readText。
         * 文本可为空、最多 65536 UTF-16 单元，无 NUL 或孤立代理项。文件最多 64 项，
         * 每项非空有效 Unicode 且无 NUL，总 UTF-16 最多 65536；[] 合法，不 trim 或验证路径。
         * 不支持格式抛 unsupported_clipboard_format，不查询或打开路径。
         */
        readData(): ScriptClipboardDataSnapshot;
        /** 替换全部格式。独立 CLI 不支持选项并返回 void；应用接入返回写入回执。 */
        writeText(text: string, options?: {excludeFromHistory?: boolean}): void | {written: true; sequenceNumber: number; writeId?: string};
    };
    /** 系统接纳请求，不等于网页或文件已经打开完成。 */
    open(target: string): void;
}>;
