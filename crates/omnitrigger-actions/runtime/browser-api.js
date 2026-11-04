// 运行时共用的浏览器 API；调用适配器决定同步或 Promise 返回。
function createBrowserApi(invoke, wrap = fn => fn) {
  function invalid(code, message, method) {
    throw Object.assign(new Error(message), {code, effectState:'not_started', method:typeof method === 'string' && method.length <= 256 ? method : 'browser.call'});
  }
  function optionsFor(options, method) {
    if (!options || typeof options !== 'object' || Array.isArray(options)
        || Object.keys(options).some(key => !['profile','timeoutMs'].includes(key))
        || (options.profile !== undefined && (typeof options.profile !== 'string' || !options.profile || options.profile.length > 128))
        || (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60000))) {
      invalid('invalid_input','浏览器选项仅支持 profile 和 1–60000 毫秒的 timeoutMs。',method);
    }
  }
  function jsonArgument(value, method, seen = new Set(), depth = 0) {
    if (depth > 64) invalid('invalid_request','浏览器参数嵌套超过 64 层。',method);
    if (typeof value === 'string') {
      if (/[\ud800-\udfff]/.test(value)) for (let i=0; i<value.length; i++) {
        const unit=value.charCodeAt(i);
        if (unit>=0xd800 && unit<=0xdbff) {
          const next=value.charCodeAt(++i);
          if (!(next>=0xdc00 && next<=0xdfff)) invalid('invalid_request','浏览器参数需要有效 Unicode。',method);
        } else if (unit>=0xdc00 && unit<=0xdfff) invalid('invalid_request','浏览器参数需要有效 Unicode。',method);
      }
      return;
    }
    if (value === null || typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) return;
    if (!value || typeof value !== 'object' || seen.has(value)
        || (!Array.isArray(value) && ![Object.prototype,null].includes(Object.getPrototypeOf(value)))
        || Object.getOwnPropertySymbols(value).length) {
      invalid('invalid_request','浏览器参数需要 JSON，不支持 undefined、函数、循环引用或非 JSON 对象。',method);
    }
    seen.add(value);
    for (const item of Array.isArray(value) ? value : Object.values(value)) jsonArgument(item,method,seen,depth+1);
    seen.delete(value);
  }
  const call = wrap((method, args = [], options = {}) => {
    if (typeof method !== 'string' || method.length > 256 || method.startsWith('chrome.')
        || !/^[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+$/.test(method)
        || method.split('.').some(part => ['__proto__','prototype','constructor'].includes(part))) {
      invalid('invalid_request','方法需使用 bookmarks.getTree 等 Chrome API 路径。',method);
    }
    if (!Array.isArray(args)) invalid('invalid_request','浏览器 args 必须为 JSON 数组。',method);
    jsonArgument(args,method);
    optionsFor(options,method);
    return invoke(method,args,options,false);
  });
  function chrome(parts = []) {
    return new Proxy(function () {}, {
      get(_target, name) {
        if (name === 'then' || name === 'toJSON' || typeof name !== 'string') return undefined;
        return chrome([...parts,name]);
      },
      apply(_target, _this, args) { return call(parts.join('.'),args); },
    });
  }
  return Object.freeze({call,profiles:wrap((options = {}) => {
    optionsFor(options,'browser.profiles');
    return invoke(undefined,undefined,options,true);
  }),chrome:chrome()});
}
