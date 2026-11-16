/// <reference path="./sdk/script-types.d.ts" />
import {runSizeStep} from './flow.mjs';
/** @param {ScriptContext<import('./types.js').SizeInput>} ctx @returns {import('./types.js').SizeResult} */
export function main(ctx){return /** @type {import('./types.js').SizeResult} */(runSizeStep(ctx,false,'decrease'));}
