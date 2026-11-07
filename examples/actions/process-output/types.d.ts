/// <reference path="./sdk/script-types.d.ts" />
import type {ActionContext, NotifyOptions, NotifyResult} from './sdk/api.js';

/** 与原宿主声明保持一致；共享配方只约束参数和结果，不改变运行时。 */
export type NodeContext = ActionContext;
export type SyncContext = ScriptContext<Record<string, unknown>>;
export type ProcessResult = Awaited<ReturnType<ActionContext['process']['capture']>>;
export type NotificationReceipt = NotifyResult;
export interface Task<Capture extends boolean = boolean> {
  program: string;
  args: string[];
  cwd: string;
  allowNonZero: boolean;
  trimOutput: boolean;
  timeoutMs: number;
  capture: Capture;
}
export interface Notification {
  params: NotifyOptions;
  summary: {
    completed: boolean;
    exitCode: number;
    stdoutUtf16Units: number;
    stderrUtf16Units: number;
  };
}
