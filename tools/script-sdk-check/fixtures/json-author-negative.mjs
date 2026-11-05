import {prepareSelection,selectionError} from './package/select-core.mjs';
/** @type {import('./package/json-types').SelectionInput} */
const invalidPath={mode:'inspect',text:'{}',path:[{}],format:'compact'};
const result=prepareSelection({mode:'inspect',text:'null',path:[],format:'compact'});
result.summary.selectedNodeCount='not-a-number';
const fault=selectionError({},false);
fault.effectState='completed';
/** @returns {import('./package/json-types').JsonActionResult} */
function invalidSyncReturn() {return Promise.resolve({status:'inspected'});}
/** @param {import('./package/json-types').SyncContext} ctx */
function invalidSyncInput(ctx) {return ctx.input.unchecked;}
/** @param {import('./package/json-types').NodeContext} ctx */
function invalidNodeInput(ctx) {return ctx.input.unchecked;}
