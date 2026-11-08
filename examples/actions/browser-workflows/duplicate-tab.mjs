import {runDuplicateTab} from './workflows.mjs';
/** @param {Pick<import('./sdk/api').ActionContext,'input'|'browser'>} ctx */
export default ctx => runDuplicateTab(ctx);
