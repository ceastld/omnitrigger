import {runRestoreSessionSync} from './workflows.mjs';
/** @param {ScriptContext} ctx @returns {ScriptJsonValue} */
export function main(ctx) { return runRestoreSessionSync(ctx); }
