import {runSizeStep} from './flow.mjs';
/** @param {import('./sdk/api.js').ActionContext} ctx */
export default async function main(ctx){return runSizeStep(ctx,true,'decrease');}
