import {basename} from 'node:path';
import {cleanText} from '../shared/text.mjs';

/** @param {import('../sdk/api.js').ActionContext} ctx */
export default async function main(ctx) {
  const target = await ctx.desktop.target();
  return {pid:process.pid,fileName:basename(cleanText(ctx.input.path)),processName:target.processName};
}
