import {runDuplicateTabSync} from './workflows.mjs';
/** @param {ScriptContext<import('./workflows.mjs').DuplicateInput>} ctx @returns {ScriptJsonValue} */
export function main(ctx) { return runDuplicateTabSync(ctx); }
