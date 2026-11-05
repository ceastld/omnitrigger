import {options, selected, submitted, choose, apply, sanitized} from './menu.mjs';

export default async function main(ctx) {
  let method = 'script.parameters';
  try {
    const params = options(ctx.input);
    method = 'ui.choose';
    const operation = selected(await choose(ctx, params));
    if (operation === null) return {status:'cancelled',submitted:false};
    method = 'window.action';
    return submitted(operation, await apply(ctx, operation));
  } catch (error) { throw sanitized(error, method); }
}
