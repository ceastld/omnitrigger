// 独立历史接口工厂；只有显式桥接才访问应用历史，不读取系统剪贴板或建立监听。
function createClipboardHistoryApi(invoke) {
  const maxU64 = '18446744073709551615';
  const methods = ['history.list', 'history.get', 'history.append', 'history.remove'];
  // 错误代码也不接受任意正文；未约定的后端代码归为 history_failed。
  const errorCodes = new Set(['invalid_input', 'invalid_response', 'cancelled', 'timeout',
    'history_unavailable', 'history_failed', 'history_item_missing', 'history_busy', 'history_corrupt',
    'history_conflict', 'history_io', 'history_id_exhausted', 'history_revision_exhausted',
    'native_bridge_unavailable', 'native_bridge_failed', 'desktop_unavailable', 'desktop_busy',
    'desktop_call_limit', 'desktop_connection', 'desktop_protocol']);
  function fault(method, code, effectState) {
    return Object.assign(new Error('剪贴板历史调用失败；请检查错误代码。'), {code, method, effectState});
  }
  function decimal(value, nonzero) {
    return typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value)
      && (!nonzero || value !== '0')
      && (value.length < maxU64.length || value <= maxU64);
  }
  function unicode(value, maximum) {
    if (typeof value !== 'string' || value.length > maximum || value.includes('\0')) return false;
    for (let i = 0; i < value.length; i++) {
      const unit = value.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = value.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
    }
    return true;
  }
  function fields(value, names) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      && Reflect.ownKeys(value).length === names.length
      && names.every(name => Object.hasOwn(value, name));
  }
  function input(method, args) {
    try {
      if (method === 'history.list') {
        const options = args[0] === undefined ? {} : args[0];
        if (args.length > 1 || options === null || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => key !== 'limit')) throw new Error();
        const {limit = 100} = options;
        if (!Number.isInteger(limit) || limit < 1 || limit > 200) throw new Error();
        return {limit};
      }
      const key = method === 'history.append' ? 'text' : 'id';
      if (args.length !== 1 || !fields(args[0], [key])) throw new Error();
      const value = args[0][key];
      if (key === 'text' ? !unicode(value, 65536) : !decimal(value, true)) throw new Error();
      return {[key]: value};
    } catch (_) { throw fault(method, 'invalid_input', 'not_started'); }
  }
  function call(method, args) {
    const params = input(method, args);
    const write = method === 'history.append' || method === 'history.remove';
    const effect = write ? 'unknown' : 'not_started';
    const invalid = () => fault(method, 'invalid_response', effect);
    const failure = error => {
      let code = 'history_failed';
      let effectState = effect;
      try {
        const reportedCode = error?.code;
        if (errorCodes.has(reportedCode)) code = reportedCode;
        if (code === 'native_bridge_unavailable' || code === 'desktop_unavailable') code = 'history_unavailable';
        if (write && error?.effectState === 'not_started') effectState = 'not_started';
      } catch (_) { /* 不回显异常对象或 getter 内容。 */ }
      throw fault(method, code, effectState);
    };
    const result = value => {
      try {
        const names = method === 'history.list' ? ['items', 'revision']
          : method === 'history.get' ? ['id', 'text', 'createdAt', 'revision']
          : method === 'history.append' ? ['id', 'revision', 'inserted'] : ['removed', 'revision'];
        if (!fields(value, names)) throw new Error();
        const revision = value.revision;
        if (!decimal(revision, false)) throw new Error();
        if (method === 'history.list') {
          const source = value.items;
          if (!Array.isArray(source) || source.length > params.limit
              || Reflect.ownKeys(source).length !== source.length + 1) throw new Error();
          const items = [];
          const ids = new Set();
          let totalUnits = 0;
          for (let i = 0; i < source.length; i++) {
            if (!Object.hasOwn(source, i)) throw new Error();
            const item = source[i];
            if (!fields(item, ['id', 'preview', 'utf16Units', 'createdAt'])) throw new Error();
            const {id, preview, utf16Units, createdAt} = item;
            if (!decimal(id, true) || ids.has(id) || !unicode(preview, 64)
                || /[\n\r\v\f\u0085\u2028\u2029]/.test(preview)
                || !Number.isInteger(utf16Units) || utf16Units < 0 || utf16Units > 65536
                || preview.length > utf16Units || totalUnits + utf16Units > 1048576
                || !unicode(createdAt, Infinity)) throw new Error();
            ids.add(id);
            totalUnits += utf16Units;
            items.push({id, preview, utf16Units, createdAt});
          }
          return {items, revision};
        }
        if (method === 'history.remove') {
          const removed = value.removed;
          if (typeof removed !== 'boolean') throw new Error();
          return {removed, revision};
        }
        const id = value.id;
        if (!decimal(id, true) || (method === 'history.get' && id !== params.id)) throw new Error();
        if (method === 'history.append') {
          const inserted = value.inserted;
          if (typeof inserted !== 'boolean') throw new Error();
          return {id, revision, inserted};
        }
        const {text, createdAt} = value;
        if (!unicode(text, 65536) || !unicode(createdAt, Infinity)) throw new Error();
        return {id, text, createdAt, revision};
      } catch (_) { throw invalid(); }
    };
    if (typeof invoke !== 'function') throw fault(method, 'history_unavailable', 'not_started');
    try {
      const value = invoke(method, params);
      return value && typeof value.then === 'function' ? value.then(result, failure) : result(value);
    } catch (error) { return failure(error); }
  }
  return Object.freeze(Object.fromEntries(methods.map(method => [method.slice(8), (...args) => call(method, args)])));
}
