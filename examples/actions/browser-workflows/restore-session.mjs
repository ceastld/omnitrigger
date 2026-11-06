import { runWorkflow } from './workflows.mjs';
/** @param {import('../../../crates/omnitrigger-actions/runtime/api').ActionContext} ctx */
export default ctx => runWorkflow(ctx, 'restore-session');
