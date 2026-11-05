import {prepare, invoke, redact} from './workflow.mjs';

export default async function main(ctx) {
  let step;
  try { step = prepare(ctx.input); }
  catch (error) { throw redact(error, 'script.parameters'); }
  while (!step.done) {
    try { step = step.resume(await invoke(ctx, step)); }
    catch (error) { throw redact(error, step.method, step.effectful); }
  }
  return step.value;
}
