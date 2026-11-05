import {prepare, invoke, redact} from './workflow.mjs';

/** @param {import('./history-types').SyncContext} ctx @returns {import('./history-types').ActionResult} */
export function main(ctx) {
  /** @type {import('./history-types').Step} */
  let step;
  try { step = prepare(ctx.input); }
  catch (error) { throw redact(error, 'script.parameters'); }
  while (!step.done) {
    try { step = step.resume(invoke(ctx, step)); }
    catch (error) { throw redact(error, (/** @type {import('./history-types').Call} */ (step)).method, (/** @type {import('./history-types').Call} */ (step)).effectful); }
  }
  return step.value;
}
