import {runDuplicateTabSync} from './workflows.mjs';
/** @type {ScriptMain<import('./workflows.mjs').DuplicateInput>} */
export function main(ctx) { return runDuplicateTabSync(ctx); }
