// 运行时共用的浏览器 API；调用适配器决定同步或 Promise 返回。
function createBrowserApi(invoke, wrap = fn => fn, invokeAsync) {
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
    return {...options};
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
      return value;
    }
    if (value === null || typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) return value;
    if (!value || typeof value !== 'object' || seen.has(value)
        || (!Array.isArray(value) && ![Object.prototype,null].includes(Object.getPrototypeOf(value)))
        || Object.getOwnPropertySymbols(value).length) {
      invalid('invalid_request','浏览器参数需要 JSON，不支持 undefined、函数、循环引用或非 JSON 对象。',method);
    }
    seen.add(value);
    // 校验时取得独立JSON值；连接等待及适配器写入都不能改变调用方的参数。
    const copy = Array.isArray(value) ? [] : {};
    if (Array.isArray(value)) {
      for (const item of value) copy.push(jsonArgument(item,method,seen,depth+1));
    } else {
      for (const [key,item] of Object.entries(value)) {
        Object.defineProperty(copy,key,{value:jsonArgument(item,method,seen,depth+1),enumerable:true,writable:true,configurable:true});
      }
    }
    seen.delete(value);
    return copy;
  }
  function callThrough(dispatch, method, args = [], options = {}) {
    if (typeof method !== 'string' || method.length > 256 || method.startsWith('chrome.')
        || !/^[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+$/.test(method)
        || method.split('.').some(part => ['__proto__','prototype','constructor'].includes(part))) {
      invalid('invalid_request','方法需使用 bookmarks.getTree 等 Chrome API 路径。',method);
    }
    if (!Array.isArray(args)) invalid('invalid_request','浏览器 args 必须为 JSON 数组。',method);
    const snapshotOptions = optionsFor(options,method);
    const snapshotArgs = jsonArgument(args,method);
    return dispatch(method,snapshotArgs,snapshotOptions,false);
  }
  const call = wrap((method, args = [], options = {}) => callThrough(invoke,method,args,options));
  function asyncDispatch(method, args, options, profiles) {
    if (typeof invokeAsync !== 'function') {
      invalid('unsupported_runtime_feature','此宿主未配置异步浏览器能力。',profiles ? 'browser.profiles' : method);
    }
    const result = invokeAsync(method,args,options,profiles);
    // 回调必须是真正的异步通道；工厂绝不调用同步invoke作为回退。
    if (!result || typeof result.then !== 'function') {
      throw Object.assign(new Error('异步浏览器适配器未返回Promise。'), {
        code:'invalid_response',effectState:'unknown',method:profiles ? 'browser.profiles' : method,
      });
    }
    return result;
  }
  const callAsync = async (method, args = [], options = {}) => callThrough(asyncDispatch,method,args,options);
  const profilesAsync = async (options = {}) => asyncDispatch(undefined,undefined,optionsFor(options,'browser.profiles'),true);
  function chrome(parts = []) {
    return new Proxy(function () {}, {
      get(_target, name) {
        if (name === 'then' || name === 'toJSON' || typeof name !== 'string') return undefined;
        return chrome([...parts,name]);
      },
      apply(_target, _this, args) { return call(parts.join('.'),args); },
    });
  }
  return Object.freeze({call,callAsync,profilesAsync,profiles:wrap((options = {}) => {
    return invoke(undefined,undefined,optionsFor(options,'browser.profiles'),true);
  }),chrome:chrome()});
}
