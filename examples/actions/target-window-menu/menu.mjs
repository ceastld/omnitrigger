const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fields = (value, names) => object(value) && Reflect.ownKeys(value).length === names.length
  && names.every(name => Object.hasOwn(value, name));
const operations = ['minimize','toggle_maximize','close'];

function failure(code) { throw Object.assign(new Error('窗口菜单参数或返回值无效。'), {code}); }

export function options(input) {
  if (!fields(input, [])) failure('invalid_input');
  return {title:'操作原目标窗口',items:[
    {id:'minimize',label:'最小化'},
    {id:'toggle_maximize',label:'切换最大化'},
    {id:'close',label:'关闭',description:'应用可能显示保存提示'},
  ]};
}

export function selected(choice) {
  if (fields(choice, ['status','reason']) && choice.status === 'cancelled'
      && ['dismissed','timeout'].includes(choice.reason)) return null;
  if (!fields(choice, ['status','id']) || choice.status !== 'selected' || !operations.includes(choice.id)) failure('invalid_response');
  return choice.id;
}

export function submitted(operation, result) {
  if (!fields(result, ['submitted']) || result.submitted !== true) failure('invalid_response');
  return {status:'submitted',operation,submitted:true};
}

export function choose(ctx, params) {
  if (typeof ctx.ui?.choose !== 'function') failure('ui_unavailable');
  return ctx.ui.choose(params);
}

export function apply(ctx, operation) {
  if (typeof ctx.desktop?.windowAction !== 'function') failure('desktop_unavailable');
  return ctx.desktop.windowAction(operation);
}

export function sanitized(error, method) {
  const codes = ['invalid_input','invalid_response','ui_unavailable','ui_busy','ui_error','desktop_unavailable',
    'native_bridge_unavailable','target_changed','target_unavailable','cancelled','timeout','window_error','window_action_failed'];
  let code = 'window_menu_failed';
  let effectState = method === 'window.action' ? 'unknown' : 'not_started';
  try {
    const reported = error?.code;
    if (codes.includes(reported)) code = reported;
    if (['not_started','unknown'].includes(error?.effectState)) effectState = error.effectState;
    if (['ui_unavailable','desktop_unavailable'].includes(code)) effectState = 'not_started';
  } catch (_) { /* 不回显异常 getter 的正文。 */ }
  return Object.assign(new Error('窗口菜单未完成，请查看错误代码。'), {code,method,effectState});
}
