import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
import closeTab from '../examples/actions/browser-close/close-tab.mjs';
import {runWorkflow} from '../examples/actions/browser-workflows/workflows.mjs';

declare const ctx: ActionContext;
// 浏览器配方对照当前正式 SDK；键盘包使用独立作者工程检查自己的声明。
const close = closeTab(ctx);
const restore = runWorkflow(ctx, 'restore-session');
void close; void restore;
