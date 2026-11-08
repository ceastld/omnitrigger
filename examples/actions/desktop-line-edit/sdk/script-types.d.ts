type ScriptPointingRootHandoffOptions=Readonly<{validForMs:number;confirmTimeoutMs:number}>;
/** requested仅表示请求发生；只有接纳且及时确认才给出现有CapturedTarget，失败不重试。 */
type ScriptPointingRootHandoffResult=
  |Readonly<{requested:true;requestAccepted:true;foregroundConfirmed:true;target:ScriptCapturedTarget;reason:null;effectState:'unknown'}>
  |Readonly<{requested:true;requestAccepted:false;foregroundConfirmed:boolean;target:null;reason:'os_rejected';effectState:'unknown'}>
  |Readonly<{requested:true;requestAccepted:true;foregroundConfirmed:false;target:null;reason:'confirmation_timeout';effectState:'unknown'}>;

/** 调用时观察身份，不是CapturedTarget输入租约，不含任何写入方法。 */
type ScriptPointingWindowIdentity=Readonly<{window_handle:string;process_id:number;thread_id:number;process_created_file_time:string}>;
type ScriptPointingRootObservation=Readonly<{
  sampledAt:'call-time';coordinateSpace:'physical-screen';point:Readonly<{x:number;y:number}>;
  inputSource:Readonly<{kind:'polled-pointer';injected:null;selfInjected:null}>;
  pointingWindow:ScriptPointingWindowIdentity;rootWindow:ScriptPointingWindowIdentity;
}>;

declare const scriptRaiShotResidentBrand: unique symbol;
type ScriptRaiShotResident = Readonly<{readonly [scriptRaiShotResidentBrand]:true;window_handle:string;process_id:number;thread_id:number;process_created_file_time:string;expires_at_unix_ms:number}>;
type ScriptExplorerContextSnapshot = Readonly<{currentDirectory:string|null;selectedPaths:readonly string[]}>;
type ScriptSelectionTextSnapshot = Readonly<{text:string;truncated:boolean}>;
declare const scriptCapturedTargetBrand: unique symbol;
type ScriptCapturedTarget = Readonly<{
    capture_id:number;
    context:Readonly<{
        window_handle:string;
        process_id:number;
        thread_id:number;
        process_created_file_time:string;
        expires_at_unix_ms:number;
    }>;
    [scriptCapturedTargetBrand]:true;
}>;
interface ScriptCaptureForegroundOptions {
    expectedProcessId:number;
    expectedProcessCreatedFileTime:string;
    validForMs:number;
}

interface ScriptVolumeAdjustOptions {
  /** 非零整数，-100至100个百分点；运行时校验范围。 */
  deltaPercent:number;
}
/** 默认多媒体输出端点；独立读回不保证与请求相同，不返回设备标识。 */
type ScriptVolumeAdjustResult = {
  effectState:'unknown';
  previousPercent:number;
  requestedPercent:number;
  currentPercent:number;
};

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
/** 标准同步入口。@type {ScriptMain} 检查上下文与返回值，也可用 ScriptMain<MyInput> 检查输入字段；不校验运行时参数。 */
type ScriptMain<Input = Record<string, any>> = (ctx: ScriptContext<Input>) => ScriptJsonValue;

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
interface ScriptDirectoryReadOptions {
    maxEntries?: number;
    order?: 'name' | 'native';
    metadata?: boolean;
    entryType?: 'all' | 'file' | 'directory';
    searchPattern?: string;
    /** 显式Windows Find匹配；不等于现代.NET托管匹配。 */
    matchMode?: 'windows-find';
}
/** UTC字段为1601纪元FILETIME；LocalTicks为0001纪元本地DateTime。均为100ns十进制字符串，不转Number。 */
type ScriptDirectoryMetadata = Readonly<{ordinal:number;attributes:number;
    creationTimeUtc100ns:string;lastAccessTimeUtc100ns:string;lastWriteTimeUtc100ns:string;
    creationTimeLocalTicks100ns:string;lastAccessTimeLocalTicks100ns:string;lastWriteTimeLocalTicks100ns:string;timeZoneKey:string}>;
type ScriptLocalDirectoryMetadataResult = Readonly<{entries:ReadonlyArray<ScriptLocalDirectoryEntry & {readonly metadata:ScriptDirectoryMetadata}> ;complete:true}>;

type ScriptLocalDirectoryEntry = Readonly<{
  name: string;
  /** 保留调用者的目录原文，仅在需要时追加反斜杠及子项名。 */
  path: string;
  kind: 'file' | 'directory' | 'other' | 'unavailable';
  reason: null | 'reparse_point' | 'cloud_placeholder' | 'cloud_path' | 'unsupported_name';
}>;
type ScriptLocalDirectoryResult = Readonly<{entries: ReadonlyArray<ScriptLocalDirectoryEntry>; complete: true}>;
interface ScriptDirectoryReadFault extends Error {
  code: 'invalid_input' | 'path_not_local' | 'path_unavailable' | 'path_missing' | 'not_directory' | 'access_denied'
    | 'directory_too_large' | 'directory_changed' | 'fs_error' | 'fs_unavailable' | 'fs_busy' | 'cancelled' | 'timeout';
  method: 'fs.read_directory';
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

/** 物理屏幕像素；右/下边界为left+width/top+height，不含该边界像素。 */
type ScriptWindowRect = Readonly<{left:number;top:number;width:number;height:number}>;
type ScriptWindowGeometry = Readonly<{coordinateSpace:'physical-screen';/** 原始类名1–255 UTF-16单元，随同目标/Lease复验；匹配属于调用方业务。 */className:string;outerRect:ScriptWindowRect;visibleFrameRect:ScriptWindowRect;
  monitorRect:ScriptWindowRect;workArea:ScriptWindowRect;primaryScreen:Readonly<{width:number;height:number}>;windowDpi:number;maximized:boolean}>;

type ScriptPointerButtonState='down'|'not_down_or_unavailable';
type ScriptPointerState=Readonly<{coordinateSpace:'physical-screen';x:number;y:number;buttons:Readonly<{left:ScriptPointerButtonState;right:ScriptPointerButtonState;middle:ScriptPointerButtonState}>;inputSource:Readonly<{kind:'polled-key-state';injected:null;selfInjected:null}>}>;
type ScriptWindowDragOptions=Readonly<{mode:'move'|'resize';button:'left'|'right'|'middle';maxDurationMs:number;maxUpdates:number;
  snap?:Readonly<{bounds:ScriptWindowRect;distance:number;waitMs:number;initialElapsedMs:number}>|null}>;
type ScriptWindowDragStopped=Readonly<{stopped:true;reason:'button_not_down_or_unavailable'|'duration_limit'|'update_limit';acceptedUpdates:number;pollSamples:number;effectState:'not_started'|'unknown';dragCompleted:false}>;

interface ScriptStopOtherInstancesFault extends Error {
  code:'invalid_input'|'actions_unavailable'|'actions_busy'|'actions_call_limit'|'action_inactive'|'action_policy_conflict'|'action_busy'|'stop_timeout'|'timeout'|'cancelled'|'invalid_response'|'actions_failed';
  method:'actions.stop_other_instances';
  effectState:'not_started'|'unknown';
}
type ScriptStopOtherInstancesResult = Readonly<{stoppedCount:0;effectState:'not_started'} | {stoppedCount:1|2|3;effectState:'unknown'}>;
type ScriptActionStateJson = null | boolean | number | string | ScriptActionStateJson[] | {[key:string]:ScriptActionStateJson};
type ScriptActionStateEntry = Readonly<{present:false} | {present:true;value:ScriptActionStateJson}>;
type ScriptActionStateSnapshot = Readonly<{revision:string;entry:ScriptActionStateEntry}>;
type ScriptActionStateCas = Readonly<{status:'committed'|'conflict';current:ScriptActionStateSnapshot}>;
interface ScriptActionStateFault extends Error {
  code:'invalid_input'|'state_unavailable'|'state_busy'|'state_limit'|'state_corrupt'|'state_storage_changed'|'state_io'|'state_failed'|'action_inactive'|'cancelled'|'timeout'|'invalid_response';
  method:'state.read'|'state.compare_and_swap';
  effectState:'not_started'|'unknown';
}
interface ScriptActionStateApi {
  /** 仅当前真实package/action；missing不补默认。session重启清空，durable需应用显式宿主。 */
  read(area:'session'|'durable',key:string): ScriptActionStateSnapshot;
  /** 小JSON，值<=16KiB，深度<=8，每区域最多64键；冲突不重试。 */
  compareAndSwap(area:'session'|'durable',key:string,expectedRevision:string,next:ScriptActionStateEntry): ScriptActionStateCas;
}

type ScriptContext<Input = Record<string, any>> = Readonly<{
    state: ScriptActionStateApi;
    /** 同步等待同包同动作其他调用真实资源清理；排除自身。无服务作用域明确不可用。 */
    actions: {stopOtherInstances(options?: {timeoutMs?:number}): ScriptStopOtherInstancesResult};

    apiVersion: 1;
    input: Input;
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
        /** 直接子项，默认 128、最多 1024；回执最多 64 KiB，超限整次拒绝。
         * 不递归、不跟随链接、不读取内容；complete 只表示读到枚举末尾，不是原子快照。
         * 使用与 stat 相同的本地路径守卫。OS 调用不能硬性抢占；迟到结果丢弃。 */
        readDirectory(path: string, options: ScriptDirectoryReadOptions & {metadata:true}): ScriptLocalDirectoryMetadataResult;
    readDirectory(path: string, options?: ScriptDirectoryReadOptions): ScriptLocalDirectoryResult;
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
        /** 更新本次动作自己的版本提示；默认列内文字；topRight为右上角绿色透明底角标，均1–16 UTF-16单位。生命周期沿用有限结果历史。回执确认模型已更新并请求刷新，不表示动作成功或像素已呈现。 */
        setActionHint(options: {text: string; placement?: 'inline' | 'topRight'}): {updated: true};
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
        /** 本次原Explorer目标，最多32个完整文件系统路径；单条1024、总计8192 UTF-16，超量拒绝。空选区为[]；虚拟目录为null；虚拟选中项报错。不复制、不读剪贴板、不补当前目录。 */
        explorerContext(): ScriptExplorerContextSnapshot;
        /** 原目标单选区；成功确认无选区/空文本时为 {text:"",truncated:false}，provider/目标/读取失败仍报错。默认 reject，显式 truncate 按请求上限取 UTF-16 前缀；拆开代理对拒绝。
         * maxUtf16Units 为1–8192整数，默认128；仅请求上限+1，helper采集及动作结果64KiB界限不变。 */
        readSelectionText(options?: {overflow?: 'reject' | 'truncate'; maxUtf16Units?: number}): ScriptSelectionTextSnapshot;
        getPointingRoot(): ScriptPointingRootObservation;
        /** 只请求一次当前指向且身份匹配的根窗；原desktop目标保持不变。 */
        handoffPointingRoot(expected: ScriptPointingRootObservation, options: ScriptPointingRootHandoffOptions): ScriptPointingRootHandoffResult;
        /** 显式CapturedTarget路径，原默认目标不变；每次复验租约/期限/身份及B前台。 */
        getWindowGeometryFor(target: ScriptCapturedTarget): ScriptWindowGeometry;
        setWindowRectFor(target: ScriptCapturedTarget, rect: ScriptWindowRect, options: {frame:'outer'|'visible'}): {submitted:true;effectState:'unknown'};
        restoreWindowFor(target: ScriptCapturedTarget): {submitted:true};
        /** 同步；显式B租约守卫与Node相同，仅确认提交。 */
        toggleMaximizeWindowFor(target: ScriptCapturedTarget): {submitted:true};
        trackWindowDragFor(target: ScriptCapturedTarget, options: ScriptWindowDragOptions): ScriptWindowDragStopped;
        /** 显式PID与创建时间；1–5000ms；仅捕获该进程当前前台，不激活或枚举。 */
        captureForegroundTarget(options: ScriptCaptureForegroundOptions): ScriptCapturedTarget;
        /** 原desktop目标不变；每次复核身份/前台/取消/期限；仅确认提交。 */
        sendShortcutTo(target: ScriptCapturedTarget, keys: number[], options?: {keyDownDelayMs?:number}): {submitted:true};
        sendTextTo(target: ScriptCapturedTarget, text: string): {submitted:true};
        adjustVolume(options: ScriptVolumeAdjustOptions): ScriptVolumeAdjustResult;
        /** 同步，参数与Node入口共用严格工厂；逐项检查回执。 */
        stepBrightness(options: ScriptBrightnessStepOptions): ScriptBrightnessStepResult;
        /** 本次提交时捕获的目标，不在调用时重新选择前台；独立 CLI 不提供目标。 */
        target(): {processName: string; windowRole: 'unknown' | 'application' | 'desktop' | 'taskbar'; title: string; processId: number; isForeground: boolean};
        /** 30 秒内可提交的原目标快照，句柄和 FILETIME 保持字符串精度。 */
        targetContext(): {window_handle: string; process_id: number; thread_id: number; process_created_file_time: string; expires_at_unix_ms: number};
        /** VK 整数，修饰键在前。仅确认系统接纳输入。 */
        sendShortcut(keys: number[], options?: {keyDownDelayMs?: number; timing?: never} | {timing: ScriptShortcutTiming; keyDownDelayMs?: never}): {submitted: true};
        sendKeySequence(modifiers: number[], keys: number[], options?: {keyDownDelayMs?: number}): {submitted: true};
        /** 只查固定隐藏常驻类，返回本ctx的一次性5秒身份快照；不读取依赖目录，不启动进程，不认证程序版本。 */
        findRaiShotResident(): ScriptRaiShotResident | null;
        /** 复验身份后投递固定WM_APP+3；只确认入队，不确认截图；身份复验与PostMessage非原子，未知结果不重试。 */
        requestRaiShotRegion(resident:ScriptRaiShotResident): {queued:true;screenshotCompleted:false};
        /** 显式跨进程Unicode标准Edit适配；必选adapter，不猜控件或回退。最多8192 UTF-16单元，不含NUL。
         * 单条EM_REPLACESEL；仅确认消息处理，不确认正文接受；不与UIA读取原子绑定，超时/写后取消为unknown。 */
        win32EditReplaceSelection(text: string, options: {adapter:'native-edit'|'winforms-edit'}): {messageProcessed:true;effectState:'unknown'};
        /** 有效 Unicode，无 NUL，最多 65,536 个 UTF-16 单元，不经过剪贴板。 */
        sendText(text: string): {submitted: true};
        /** 向原目标发送 Ctrl+C 并读取新文本，复制结果保留在剪贴板。 */
        copySelectionText(): string;
        /** 同步向捕获且仍为前台的原目标发送一次左 Ctrl+C，不重抓或抢焦点。
         * changed 比较最终序号与复制前序号，不证明生产者。默认未变抛 selection_unchanged。
         * 不恢复剪贴板，取消不撤销输入；客户端最多等待 8 秒，不保证抢占阻塞 Win32 调用。
         */
        copySelectionData(options?: ScriptSelectionCopyOptions): ScriptSelectionDataSnapshot;
        /** 调用时只读采样物理坐标与当前按钮；不可区分未按下和不可用。 */
        getPointerState(): ScriptPointerState;
        /** 现有worker内单次有界调用：20–2000ms，最多64次提交、201次主采样；10ms协作等待。
         * 不注入/消费输入、不还原窗口；非明确按下即停，返回停止原因而非拖拽完成。已提交不回滚。 */
        trackWindowDrag(options: ScriptWindowDragOptions): ScriptWindowDragStopped;
        /** 原捕获窗口仍可见且为前台；DWM可见边界无可用值时报错，不回退外框。
         * windowDpi是GetDpiForWindow原值，不是坐标缩放因子；工作区为窗口所在或最近显示器。 */
        getWindowGeometry(): ScriptWindowGeometry;
        /** 显式外框或可见边界；坐标i32，宽高1–65535，不裁剪工作区、不激活/改Z序、不隐式还原最大化窗口。
         * 一次异步SetWindowPos；仅确认提交，应用/DPI改变可调整结果；发起后的失败/取消/超时为unknown，无重试或补偿。 */
        setWindowRect(rect: ScriptWindowRect, options: {frame:'outer'|'visible'}): {submitted:true;effectState:'unknown'};
        /** 操作本次捕获且仍为前台的窗口；仅表示系统接纳请求，关闭可能弹出保存提示。 */
        /** restore 显式请求原窗口正常显示，不激活、不切换最大化状态；submitted 不代表已还原。 */
        windowAction(operation: 'toggle_maximize' | 'restore' | 'toggle_topmost' | 'minimize' | 'close'): {submitted: true};
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
        /** 应用桥接写Unicode CF_HDROP；1–64项绝对Windows路径，总UTF-16不超65536。
         * 不查询或打开路径，不复制磁盘文件，不粘贴；替换全部剪贴板格式且不自动恢复。
         * 清空后的失败/取消为unknown；sequenceNumber仅为观察值，不证明独占或文件存在。 */
        writeFiles(paths: readonly string[]): {written:true;sequenceNumber:number};
        /** 替换全部格式。独立 CLI 不支持选项并返回 void；应用接入返回写入回执。 */
        writeText(text: string, options?: {excludeFromHistory?: boolean}): void | {written: true; sequenceNumber: number; writeId?: string};
    };
    /** 系统接纳请求，不等于网页或文件已经打开完成。 */
    open(target: string): void;
    /** 普通本地文件；显式使用文件父目录为工作目录。复验元数据，单次提交，不等待关联程序退出。 */
    openFileInParent(target: string): void;
}>;
