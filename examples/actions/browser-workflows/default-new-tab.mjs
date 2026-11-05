import { runWorkflow } from './workflows.mjs';
/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) { return runWorkflow(ctx, 'create-default-tab'); }
