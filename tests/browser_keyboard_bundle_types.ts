import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
import type {ActionContext as CloseContext} from '../examples/actions/browser-close/sdk/api';
import type {ActionContext as WorkflowContext} from '../examples/actions/browser-workflows/sdk/api';
import closeTab from '../examples/actions/browser-close/close-tab.mjs';
import {runWorkflow} from '../examples/actions/browser-workflows/workflows.mjs';

declare const ctx: ActionContext;
declare const closeContext: CloseContext;
declare const workflowContext: WorkflowContext;
// 各作者工程使用自身SDK身份；浏览器协议仍须兼容当前正式SDK。
const closeBrowser: CloseContext['browser'] = ctx.browser;
const workflowBrowser: WorkflowContext['browser'] = ctx.browser;
const close = closeTab(closeContext);
const restore = runWorkflow(workflowContext, 'restore-session');
void closeBrowser; void workflowBrowser; void close; void restore;
