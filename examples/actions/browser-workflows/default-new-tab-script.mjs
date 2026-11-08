import { runDefaultTabSync } from './workflows.mjs';
/** @param {ScriptContext} ctx @returns {ScriptJsonValue} */
export function main(ctx) { return runDefaultTabSync(ctx); }
