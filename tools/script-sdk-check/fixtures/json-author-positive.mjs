import {prepareSelection,selectionError} from './package/select-core.mjs';
/** @type {import('./package/json-types').SelectionInput} */
export const validPath={mode:'inspect',text:'{"items":[false]}',path:['items',0],format:'compact'};
export const result=prepareSelection(validPath);
/** @type {number} */
export const count=result.summary.selectedNodeCount;
export const fault=selectionError({},false);
/** @type {import('./package/json-types').EffectState} */
export const effect=fault.effectState;
/** @returns {import('./package/json-types').JsonActionResult} */
export function validSyncReturn() {return {status:'inspected',selectedKind:'null',selectedNodeCount:1,pathLength:0,inputUtf16Units:4,outputUtf16Units:4,inputSubmitted:false};}
/** @param {import('./package/json-types').SyncContext} ctx */
export function validSyncInput(ctx) {return prepareSelection(ctx.input);}
/** @param {import('./package/json-types').NodeContext} ctx */
export function validNodeInput(ctx) {return prepareSelection(ctx.input);}
