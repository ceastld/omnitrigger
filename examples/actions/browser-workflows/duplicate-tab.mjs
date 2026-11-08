import { runWorkflow } from './workflows.mjs';
/** @param {import('./sdk/api').ActionContext} ctx */
export default ctx => runWorkflow(ctx, 'duplicate-tab');
