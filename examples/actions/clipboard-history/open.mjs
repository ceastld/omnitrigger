/** @param {import('./history-types').NodeContext} ctx @returns {Promise<{opened:true}>} */
export default async function main(ctx) { return ctx.ui.openHistory(); }
