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
  });
  const clipboard = Object.freeze({
    readText: () => invoke('clipboard.read', {}),
    writeText: (text, options = {}) => {
      if (options === null || typeof options !== 'object' || Array.isArray(options)
          || Object.keys(options).some(key => key !== 'excludeFromHistory')
          || ('excludeFromHistory' in options && typeof options.excludeFromHistory !== 'boolean')) {
        invalid('clipboard.write', '剪贴板选项只接受布尔值 excludeFromHistory。');
      }
      return textCall('clipboard.write', text, options.excludeFromHistory ? {exclude_from_history:true} : {});
    },
  });
  return {desktop, clipboard};
}
