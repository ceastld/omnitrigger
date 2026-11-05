// Node 与轻量宿主共用的桌面接口。传入桥接函数决定同步或异步执行方式。
function createDesktopApi(invoke) {
  function invalid(method, message) {
    throw Object.assign(new Error(message), {code:'invalid_input', effectState:'not_started', method});
  }
  function keyTiming(options, count, method) {
    const value = options?.keyDownDelayMs === undefined ? 0 : options.keyDownDelayMs;
    if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(key => key !== 'keyDownDelayMs')
        || !Number.isInteger(value) || value < 0 || value > 1000 || (count + 1) * value > 3000) {
      invalid(method, '按下延迟需要 0–1000ms 整数，单次等待合计不能超过 3000ms。');
    }
    return value;
  }
  function textCall(method, text, extra = {}) {
    let valid = typeof text === 'string' && text.length <= 65536 && !text.includes('\0');
    if (valid) for (let i = 0; i < text.length; i++) {
      const unit = text.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = text.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) { valid = false; break; }
      } else if (unit >= 0xdc00 && unit <= 0xdfff) { valid = false; break; }
    }
    if (!valid) invalid(method, '文本需要有效 Unicode，不含 NUL，最多 65,536 个 UTF-16 单元。');
    return invoke(method, {text, ...extra});
  }
  function dataFault(method, code, effectState) {
    return Object.assign(new Error('剪贴板或选区数据调用失败；请检查错误代码。'), {code, effectState, method});
  }
  function dataText(value, max, allowEmpty) {
    if (typeof value !== 'string' || value.length > max || (!allowEmpty && value.length === 0) || value.includes('\0')) return false;
    for (let i = 0; i < value.length; i++) {
      const unit = value.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = value.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
    }
    return true;
  }
  function exactDataFields(value, fields) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      && Reflect.ownKeys(value).length === fields.length && fields.every(key => Object.hasOwn(value, key));
  }
  function dataCall(method, params) {
    const copy = method === 'selection.copy_data';
    const effect = copy ? 'unknown' : 'not_started';
    const failure = error => {
      let code = 'native_bridge_failed';
      let notStarted = !copy;
      try {
        const reported = error?.code;
        if (typeof reported === 'string' && /^[A-Za-z0-9_]{1,64}$/.test(reported)) code = reported;
        if (copy) notStarted = error?.effectState === 'not_started';
      } catch (_) { /* 不读取或回显异常 getter 的正文。 */ }
      throw dataFault(method, code, notStarted ? 'not_started' : 'unknown');
    };
    const result = value => {
      try {
        const kind = value?.kind;
        const files = kind === 'files';
        const fields = ['kind', files ? 'paths' : 'text', 'sequenceNumber', ...(copy ? ['changed'] : [])];
        if (!exactDataFields(value, fields) || !['text','files'].includes(kind)) throw new Error();
        const sequenceNumber = value.sequenceNumber;
        const changed = copy ? value.changed : undefined;
        if (!Number.isInteger(sequenceNumber) || sequenceNumber < 0 || sequenceNumber > 0xffffffff
            || (copy && typeof changed !== 'boolean')) throw new Error();
        let data;
        if (files) {
          const source = value.paths;
          if (!Array.isArray(source) || source.length > 64
              || Reflect.ownKeys(source).length !== source.length + 1) throw new Error();
          const paths = [];
          let units = 0;
          for (let i = 0; i < source.length; i++) {
            if (!Object.hasOwn(source, i)) throw new Error();
            const path = source[i];
            if (!dataText(path, 65536 - units, false)) throw new Error();
            units += path.length;
            paths.push(path);
          }
          // 路径仅复制数据，不套用 stat、云盘、绝对路径或存在性规则，不 trim。
          data = {kind:'files', paths, sequenceNumber};
        } else {
          const text = value.text;
          if (!dataText(text, 65536, true)) throw new Error();
          data = {kind:'text', text, sequenceNumber};
        }
        return copy ? {...data, changed} : data;
      } catch (_) { throw dataFault(method, 'invalid_response', effect); }
    };
    try {
      const value = invoke(method, params);
      return value && typeof value.then === 'function' ? value.then(result, failure) : result(value);
    } catch (error) { return failure(error); }
  }
  const desktop = Object.freeze({
    target: () => invoke('target.info', {}),
    targetContext: () => invoke('target.context', {}),
    sendShortcut: (keys, options = {}) => {
      const method = 'keyboard.shortcut';
      if (!Array.isArray(keys) || keys.length < 1 || keys.length > 5
          || keys.some(key => !Number.isInteger(key) || key < 1 || key > 254)) {
        invalid(method, '快捷键需要 1–5 个明确的 VK 整数；修饰键在前。');
      }
      if (options?.timing !== undefined) {
        const timing = options.timing;
        const overrides = timing?.process_overrides ?? {};
        if (typeof options !== 'object' || Array.isArray(options) || Object.keys(options).length !== 1 || !timing || typeof timing !== 'object' || Array.isArray(timing)
            || Object.keys(timing).some(key => !['key_down_delay_ms','process_overrides'].includes(key))
            || (timing.key_down_delay_ms !== undefined && (!Number.isInteger(timing.key_down_delay_ms) || timing.key_down_delay_ms < 0 || timing.key_down_delay_ms > 1000))
            || !overrides || typeof overrides !== 'object' || Array.isArray(overrides) || Object.keys(overrides).length > 16
            || Object.values(overrides).some(value => !Number.isInteger(value) || value < 0 || value > 1000)
            || timing.process_overrides === null) {
          invalid(method, 'timing 需要原生快捷键时序对象，与 keyDownDelayMs 互斥。');
        }
        return invoke(method, {keys, timing});
      }
      const delay = keyTiming(options, 1, method);
      return invoke(method, delay ? {keys, key_down_delay_ms:delay} : {keys});
    },
    sendKeySequence: (modifiers, keys, options = {}) => {
      const method = 'keyboard.sequence';
      if (!Array.isArray(modifiers) || modifiers.length > 4 || !Array.isArray(keys) || keys.length < 1 || keys.length > 16
          || [...modifiers,...keys].some(key => !Number.isInteger(key) || key < 1 || key > 254)) {
        invalid(method, '连续按键需要 0–4 个修饰键和 1–16 个普通键，使用明确 VK 整数。');
      }
      return invoke(method, {modifiers, keys, key_down_delay_ms:keyTiming(options, keys.length, method)});
    },
    sendText: text => textCall('keyboard.text', text),
    copySelectionText: () => invoke('selection.copy', {}),
    copySelectionData: (...args) => {
      const method = 'selection.copy_data';
      let params;
      try {
        const options = args[0] === undefined ? {} : args[0];
        if (args.length > 1 || options === null || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => !['timeoutMs','pollIntervalMs','settleMs','allowUnchanged'].includes(key))) throw new Error();
        const {timeoutMs = 600, pollIntervalMs = 15, settleMs = 30, allowUnchanged = false} = options;
        if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 5000
            || !Number.isInteger(pollIntervalMs) || pollIntervalMs < 1 || pollIntervalMs > 1000 || pollIntervalMs > timeoutMs
            || !Number.isInteger(settleMs) || settleMs < 0 || settleMs > 1000 || typeof allowUnchanged !== 'boolean') throw new Error();
        params = {timeout_ms:timeoutMs,poll_interval_ms:pollIntervalMs,settle_ms:settleMs,allow_unchanged:allowUnchanged};
      } catch (_) { invalid(method, '复制选区数据只接受有效的 timeoutMs、pollIntervalMs、settleMs 和 allowUnchanged。'); }
      return dataCall(method, params);
    },
    windowAction: operation => {
      if (!['toggle_maximize', 'minimize', 'close'].includes(operation)) {
        invalid('window.action', '窗口操作需要 toggle_maximize、minimize 或 close。');
      }
      return invoke('window.action', {operation});
    },
  });
  const clipboard = Object.freeze({
    readText: () => invoke('clipboard.read', {}),
    readData: (...args) => {
      if (args.length !== 0) invalid('clipboard.read_data', 'readData 不接收参数。');
      return dataCall('clipboard.read_data', {});
    },
    writeText: (text, options = {}) => {
      if (options === null || typeof options !== 'object' || Array.isArray(options)
          || Object.keys(options).some(key => key !== 'excludeFromHistory')
          || ('excludeFromHistory' in options && typeof options.excludeFromHistory !== 'boolean')) {
        invalid('clipboard.write', '剪贴板选项只接受布尔值 excludeFromHistory。');
      }
      return textCall('clipboard.write', text, options.excludeFromHistory ? {exclude_from_history:true} : {});
    },
  });
  const ui = Object.freeze({
    toggleHistory: (...args) => {
      const method = 'ui.toggle_history';
      if (args.length !== 0) invalid(method, '切换历史窗口不接受参数。');
      const receipt = value => {
        const keys = value && typeof value === 'object' && !Array.isArray(value) ? Reflect.ownKeys(value) : [];
        if (keys.length === 1 && keys[0] === 'opened' && value.opened === true) return {opened:true};
        if (keys.length === 1 && keys[0] === 'closed' && value.closed === true) return {closed:true};
        throw Object.assign(new Error('历史窗口切换回执未确认，不会自动重试。'),
          {code:'invalid_response', method, effectState:'unknown'});
      };
      const result = invoke(method, {});
      return result && typeof result.then === 'function' ? result.then(receipt) : receipt(result);
    },
    openHistory: (...args) => {
      const method = 'ui.open_history';
      if (args.length !== 0) invalid(method, '打开历史窗口不接受参数。');
      const receipt = value => {
        if (!value || typeof value !== 'object' || Array.isArray(value)
            || Reflect.ownKeys(value).length !== 1 || value.opened !== true) {
          throw Object.assign(new Error('历史窗口创建回执未确认，不会自动重试。'),
            {code:'invalid_response', method, effectState:'unknown'});
        }
        return {opened:true};
      };
      const result = invoke(method, {});
      return result && typeof result.then === 'function' ? result.then(receipt) : receipt(result);
    },
    notify: options => {
      const method = 'ui.notify';
      const text = (value, max, multiline) => {
        // 与原生 Rust 校验共用 Unicode White_Space 定义；FEFF 是格式字符，不是空白。
        if (typeof value !== 'string' || !/[^\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/.test(value) || value.length > max
            || (multiline ? /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/ : /[\x00-\x1f\x7f-\x9f\u2028\u2029]/).test(value)) return false;
        for (let i = 0; i < value.length; i++) {
          const unit = value.charCodeAt(i);
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
          } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
      };
      if (!options || typeof options !== 'object' || Array.isArray(options)
          || Reflect.ownKeys(options).some(key => !['message','title','level','placement','durationMs'].includes(key))) {
        invalid(method, '通知参数只接受 message、title、level、placement 和 durationMs。');
      }
      const {message, title = 'OmniTrigger', level = 'info', placement = 'topRight', durationMs = 3000} = options;
      if (!text(message, 1024, true) || !text(title, 128, false)
          || !['info','success','warning','error'].includes(level)
          || !['topRight','bottomRight','center'].includes(placement)
          || !Number.isInteger(durationMs) || durationMs < 1000 || durationMs > 30000) {
        invalid(method, '通知需要 1–1024 个 UTF-16 单元的有效正文、最多 128 单元的单行标题、有效级别和位置，以及 1000–30000 毫秒的整数时长。');
      }
      // 两类宿主共享同一份纯文本参数；不裁剪、不转义或更改作者正文。
      return invoke(method, {message, title, level, placement, durationMs});
    },
    choose: options => {
      const method = 'ui.choose';
      const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
      const text = (value, max) => {
        if (typeof value !== 'string' || !value.trim() || value.length > max || /[\x00-\x1f\x7f-\x9f\u2028\u2029]/.test(value)) return false;
        for (let i = 0; i < value.length; i++) {
          const unit = value.charCodeAt(i);
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
          } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
      };
      const ids = new Set();
      if (!object(options) || Object.keys(options).some(key => !['items','title','placeholder','timeoutMs'].includes(key))
          || !Array.isArray(options.items) || options.items.length < 1 || options.items.length > 200
          || (options.title !== undefined && !text(options.title, 128))
          || (options.placeholder !== undefined && !text(options.placeholder, 128))
          || (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60000))
          || options.items.some(item => {
            if (!object(item) || Object.keys(item).some(key => !['id','label','description'].includes(key))
                || !text(item.id, 128) || !text(item.label, 256)
                || (item.description !== undefined && item.description !== '' && !text(item.description, 512)) || ids.has(item.id)) return true;
            ids.add(item.id);
            return false;
          })) invalid(method, '选择需要 1–200 个唯一 ID 的候选项和有效单行文本，时限 1–60000 毫秒。');
      // 复制白名单字段；两种引擎传递同一份参数，不传递业务对象或回调。
      const params = {items:options.items.map(item => ({id:item.id,label:item.label,...(item.description === undefined ? {} : {description:item.description})})),
        ...(options.title === undefined ? {} : {title:options.title}),
        ...(options.placeholder === undefined ? {} : {placeholder:options.placeholder}),
        ...(options.timeoutMs === undefined ? {} : {timeoutMs:options.timeoutMs})};
      let bytes = 0;
      for (const character of JSON.stringify(params)) {
        const point = character.codePointAt(0);
        bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      }
      if (bytes > 32*1024) invalid(method, '选择参数总量最多 32 KiB。');
      return invoke(method, params);
    },
  });
  const fs = Object.freeze({
    stat: (...args) => {
      const method = 'fs.stat';
      const fail = (code = 'invalid_input') => {
        throw Object.assign(new Error(code === 'path_not_local' ? '路径不属于支持的普通本地磁盘路径。' : '本地路径状态查询失败；请检查错误代码。'), {code, effectState:'not_started', method});
      };
      const path = args[0];
      if (args.length !== 1 || typeof path !== 'string' || path.length < 3 || path.length > 32767
          || !/^[A-Za-z]:[\\/]/.test(path) || /[\x00-\x1f\x7f-\x9f]/.test(path)) fail();
      for (let i = 0; i < path.length; i++) {
        const unit = path.charCodeAt(i);
        if (unit >= 0xd800 && unit <= 0xdbff) {
          const next = path.charCodeAt(++i);
          if (!(next >= 0xdc00 && next <= 0xdfff)) fail();
        } else if (unit >= 0xdc00 && unit <= 0xdfff) fail();
      }
      // 仅对组件检查副本统一分隔符，查询仍传原文。字符串 split 避免
      // Boa 的正则 split 在最大长度路径上逐位置尝试匹配。
      const components = path.length === 3 ? [] : path.slice(3).replaceAll(String.fromCharCode(92), '/').split('/');
      if (components.at(-1) === '') components.pop();
      for (const component of components) {
        if (!component || component === '.' || component === '..' || /[<>:"|?*]/.test(component) || /[ .]$/.test(component)
            || /^(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?: *\.| *$)/i.test(component)) fail();
      }
      if (components.some(component => {
        const name = component.toLowerCase();
        return name.startsWith('onedrive') || name.startsWith('icloud') || name === 'dropbox';
      })) fail('path_unavailable');
      const failure = error => {
        const codes = ['invalid_input','path_not_local','path_unavailable','access_denied','fs_error','fs_unavailable','fs_busy','cancelled','timeout'];
        fail(codes.includes(error?.code) ? error.code : 'fs_error');
      };
      const result = value => {
        if (!value || typeof value !== 'object' || Array.isArray(value) || Reflect.ownKeys(value).length !== 1
            || !Object.hasOwn(value, 'kind') || !['file','directory','missing','other'].includes(value.kind)) fail('fs_error');
        return value;
      };
      // 词法检查不访问磁盘；盘类型、物理卷、祖先属性和占位文件仍由平台独立复验。
      try {
        const value = invoke(method, {path});
        return value && typeof value.then === 'function' ? value.then(result, failure) : result(value);
      } catch (error) { return failure(error); }
    },
  });
  return {desktop, clipboard, ui, fs};
}
