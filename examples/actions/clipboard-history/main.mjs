import {prepare, invoke, redact} from './workflow.mjs';

/** @param {import('./history-types').NodeContext} ctx @returns {Promise<import('./history-types').ActionResult>} */
export default async function main(ctx) {
  /** @type {import('./history-types').Step} */
  let step;
  try { step = prepare(ctx.input); }
  catch (error) { throw redact(error, 'script.parameters'); }
  while (!step.done) {
    try { step = step.resume(await invoke(ctx, step)); }
    catch (error) { throw redact(error, (/** @type {import('./history-types').Call} */ (step)).method, (/** @type {import('./history-types').Call} */ (step)).effectful); }
  }
  return step.value;
}
