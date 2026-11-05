import { runWorkflow } from './workflows.mjs';
export default async function main(ctx) { return runWorkflow(ctx, 'create-default-tab'); }
