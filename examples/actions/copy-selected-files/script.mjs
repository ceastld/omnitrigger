/// <reference path="./sdk/script-types.d.ts" />
import { requireWriter, requireFileOrDirectory, selectedPaths } from './files.mjs';

/** @type {ScriptMain<Record<string, unknown>>} */
export function main(ctx) {
  requireWriter(ctx);
  const paths = selectedPaths(ctx.desktop.explorerContext());
  for (const path of paths) requireFileOrDirectory(ctx.fs.stat(path));
  return ctx.clipboard.writeFiles(paths);
}
