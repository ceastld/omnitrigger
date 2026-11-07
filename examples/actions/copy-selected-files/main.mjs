import { checkCancelled, requireWriter, requireFileOrDirectory, selectedPaths } from './files.mjs';

/** @param {import('./sdk/api.js').ActionContext} ctx */
export default async function (ctx) {
  requireWriter(ctx);
  checkCancelled(ctx);
  const paths = selectedPaths(await ctx.desktop.explorerContext());
  checkCancelled(ctx);
  for (const path of paths) {
    requireFileOrDirectory(await ctx.fs.stat(path));
    checkCancelled(ctx);
  }
  const receipt = await ctx.clipboard.writeFiles(paths);
  checkCancelled(ctx, true);
  return receipt;
}
