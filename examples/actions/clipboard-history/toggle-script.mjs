/** @param {import('./history-types').SyncContext} ctx @returns {{opened:true}|{closed:true}} */
export function main(ctx) { return ctx.ui.toggleHistory(); }
