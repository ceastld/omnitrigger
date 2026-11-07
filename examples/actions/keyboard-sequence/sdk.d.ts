/// <reference path="./sdk/script-types.d.ts" />

/** 包内固定声明；Node入口引用真实异步SDK，ScriptMain沿用同步SDK。 */
type OmniTriggerActionContext = import('./sdk/api').ActionContext;
