// Node与同步Script共用；invoke只能传方法及等待预算，身份由服务端作用域决定。
function createActionsApi(invoke, asyncMode = false) {
  const method = 'actions.stop_other_instances';
  const fault = (code, effectState) => Object.assign(new Error('停止其他调用未获完整确认；不会自动重试或补发输入。'), {code,effectState,method});
  function execute(...args) {
    let dispatched = false;
    const failure = cause => {
      let code = 'actions_failed', effect = dispatched ? 'unknown' : 'not_started';
      try {
        const known = ['invalid_input','actions_unavailable','actions_busy','actions_call_limit','action_inactive',
          'action_policy_conflict','action_busy','stop_timeout','timeout','cancelled','invalid_response'];
        if (known.includes(cause?.code)) code = cause.code;
        else if (['native_bridge_unavailable','desktop_unavailable'].includes(cause?.code)) code = 'actions_unavailable';
        else if (cause?.code === 'desktop_busy') code = 'actions_busy';
        else if (cause?.code === 'desktop_call_limit') code = 'actions_call_limit';
        if (cause?.effectState === 'not_started') effect = 'not_started';
      } catch (_) { /* 不回显异常内容或参数。 */ }
      throw fault(code,effect);
    };
    try {
      const options = args[0] === undefined ? {} : args[0];
      if (args.length > 1 || !options || typeof options !== 'object' || Array.isArray(options)
          || ![Object.prototype,null].includes(Object.getPrototypeOf(options))) throw fault('invalid_input','not_started');
      const descriptors = Object.getOwnPropertyDescriptors(options);
      if (Reflect.ownKeys(descriptors).some(key => key !== 'timeoutMs' || !Object.hasOwn(descriptors[key],'value'))) throw fault('invalid_input','not_started');
      const timeoutMs = Object.hasOwn(descriptors,'timeoutMs') ? descriptors.timeoutMs.value : 500;
      if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 2000) throw fault('invalid_input','not_started');
      const checked = result => {
        if (!result || typeof result !== 'object' || Array.isArray(result)) throw fault('invalid_response','unknown');
        const fields = Object.getOwnPropertyDescriptors(result);
        if (Reflect.ownKeys(fields).length !== 2 || !['stoppedCount','effectState'].every(key => Object.hasOwn(fields,key) && Object.hasOwn(fields[key],'value'))
            || !Number.isInteger(fields.stoppedCount.value) || fields.stoppedCount.value < 0 || fields.stoppedCount.value > 3
            || fields.effectState.value !== (fields.stoppedCount.value === 0 ? 'not_started' : 'unknown')) throw fault('invalid_response','unknown');
        return Object.freeze({stoppedCount:fields.stoppedCount.value,effectState:fields.effectState.value});
      };
      dispatched = true;
      const result = invoke(method,{timeout_ms:timeoutMs});
      return asyncMode ? Promise.resolve(result).then(checked).catch(failure) : checked(result);
    } catch (cause) { return failure(cause); }
  }
  return Object.freeze({stopOtherInstances:asyncMode ? async (...args) => execute(...args) : execute});
}
