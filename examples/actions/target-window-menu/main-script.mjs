import {options, selected, submitted, choose, apply, sanitized} from './menu.mjs';

export function main(ctx) {
  let method = 'script.parameters';
  try {
    const params = options(ctx.input);
    method = 'ui.choose';
    const operation = selected(choose(ctx, params));
    if (operation === null) return {status:'cancelled',submitted:false};
    method = 'window.action';
    return submitted(operation, apply(ctx, operation));
  } catch (error) { throw sanitized(error, method); }
}
