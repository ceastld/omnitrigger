/** 作者类型声明；运行时通过 ctx.clipboard.history 调用，不导出宿主内部工厂。 */
/** u64 的规范十进制字符串；ID 非零，revision 可以为 "0"，不转换为 JS number。 */
export type HistoryDecimal = string;
/** createdAt 为应用持久化的时间字符串；SDK 不重新解释时区或重写格式。 */
export interface ClipboardHistoryItem {
  id: HistoryDecimal;
  /** 单行摘要，最多 64 UTF-16 单元，不切断代理对；不是全文。 */
  preview: string;
  utf16Units: number;
  createdAt: string;
}
export interface ClipboardHistoryList {items: ClipboardHistoryItem[]; revision: HistoryDecimal}
export interface ClipboardHistoryText {id: HistoryDecimal; text: string; createdAt: string; revision: HistoryDecimal}
export interface ClipboardHistoryAppend {id: HistoryDecimal; revision: HistoryDecimal; inserted: boolean}
export interface ClipboardHistoryRemove {removed: boolean; revision: HistoryDecimal}
export type ClipboardHistoryMethod = 'history.list' | 'history.get' | 'history.append' | 'history.remove';
export interface ClipboardHistoryFault extends Error {
  code: string;
  method: ClipboardHistoryMethod;
  /** list/get 总为 not_started；写入发出后无法确认的失败为 unknown。 */
  effectState: 'not_started' | 'unknown';
}
export interface ClipboardHistoryApi {
  /** 默认 limit=100，整数 1..200；只返回摘要，不返回正文。 */
  list(options?: {limit?: number}): ClipboardHistoryList;
  /** 不存在抛 history_item_missing；正文只供脚本内部使用，不应放进动作终态。 */
  get(options: {id: HistoryDecimal}): ClipboardHistoryText;
  /** 显式保存文本：可为空、保留空白、有效 Unicode、无 NUL，最多 65536 UTF-16。
   * 连续相同文本可去重并 inserted=false；不自动重试，不建立监听或读系统剪贴板。
   */
  append(options: {text: string}): ClipboardHistoryAppend;
  /** 不存在返回 removed=false；无变化不递增 revision。 */
  remove(options: {id: HistoryDecimal}): ClipboardHistoryRemove;
}
export type AsyncClipboardHistoryApi = {
  [Key in keyof ClipboardHistoryApi]: (...args: Parameters<ClipboardHistoryApi[Key]>) => Promise<ReturnType<ClipboardHistoryApi[Key]>>;
};
