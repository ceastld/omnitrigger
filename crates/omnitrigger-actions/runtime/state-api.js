// 纯协议薄封装，Node异步/Script同步共用。身份、缓存和存储只在父服务中。
function createStateApi(invoke, asyncMode = false) {
  const bytes = text => {
    let n = 0;
    for (const c of text) { const p = c.codePointAt(0); n += p < 128 ? 1 : p < 2048 ? 2 : p < 65536 ? 3 : 4; }
    return n;
  };
  const failure = (code, method, effectState) => Object.assign(new Error('动作状态操作未获完整确认，请检查错误代码。'), {code,method,effectState});
  const fields = value => {
    if (!value || typeof value !== 'object' || Array.isArray(value) || ![Object.prototype,null].includes(Object.getPrototypeOf(value))) throw new Error();
    const keys = Reflect.ownKeys(value);
    if (keys.length > 2048 || keys.some(k => typeof k !== 'string')) throw new Error();
    const result = Object.create(null);
    for (const k of keys) {
      const d = Object.getOwnPropertyDescriptor(value,k);
      if (!d || !Object.hasOwn(d,'value') || !d.enumerable) throw new Error();
      result[k] = d.value;
    }
    return result;
  };
  const exact = (object, keys) => Object.keys(object).length === keys.length && keys.every(k => Object.hasOwn(object,k));
  const keyValid = key => typeof key === 'string' && bytes(key) > 0 && bytes(key) <= 64 && !/[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u.test(key);
  const revisionValid = revision => typeof revision === 'string' && /^[0-9a-f]{64}$/.test(revision);
  function jsonValue(value) {
    const seen = new Set(); let nodes = 0;
    function copy(value,depth) {
      if (++nodes > 2048 || depth > 8) throw new Error();
      if (value === null || typeof value === 'boolean') return value;
      if (typeof value === 'number') {
        if (!Number.isFinite(value) || Number.isInteger(value) && !Number.isSafeInteger(value)) throw new Error();
        return value;
      }
      if (typeof value === 'string') {
        // JSON跨运行时不能保留孤立代理项。
        for (let i=0;i<value.length;i++) {
          const n=value.charCodeAt(i);
          if (n>=0xd800&&n<=0xdbff) { const low=value.charCodeAt(++i); if (!(low>=0xdc00&&low<=0xdfff)) throw new Error(); }
          else if (n>=0xdc00&&n<=0xdfff) throw new Error();
        }
        return value;
      }
      if (!value || typeof value !== 'object' || seen.has(value)) throw new Error();
      seen.add(value);
      let out;
      if (Array.isArray(value)) {
        if (value.length > 2048 || Reflect.ownKeys(value).length !== value.length+1) throw new Error();
        out=[];
        for(let i=0;i<value.length;i++) {const d=Object.getOwnPropertyDescriptor(value,String(i));if(!d||!Object.hasOwn(d,'value'))throw new Error();out.push(copy(d.value,depth+1));}
      } else {
        const f=fields(value);out={};
        for(const k of Object.keys(f)) {
          for(const c of k) {const p=c.codePointAt(0);if(p>=0xd800&&p<=0xdfff)throw new Error();}
          Object.defineProperty(out,k,{value:copy(f[k],depth+1),enumerable:true,writable:true,configurable:true});
        }
      }
      seen.delete(value);
      return out;
    }
    const result=copy(value,0);
    if(bytes(JSON.stringify(result))>16*1024)throw new Error();
    return result;
  }
  function entry(raw) {
    const f=fields(raw);
    if(f.present===false&&exact(f,['present']))return {present:false};
    if(f.present===true&&exact(f,['present','value']))return {present:true,value:jsonValue(f.value)};
    throw new Error();
  }
  function snapshot(raw) {
    const f=fields(raw);
    if(!exact(f,['revision','entry'])||!revisionValid(f.revision))throw new Error();
    return Object.freeze({revision:f.revision,entry:Object.freeze(entry(f.entry))});
  }
  function operation(method,args) {
    const cas=method==='state.compare_and_swap';let sent=false;
    const rejected=cause=>{
      let code='state_failed',effectState=cas&&sent?'unknown':'not_started';
      try {
        if(['invalid_input','state_unavailable','state_busy','state_limit','state_corrupt','state_storage_changed','state_io','state_failed','action_inactive','cancelled','timeout','invalid_response'].includes(cause?.code))code=cause.code;
        else if(['actions_unavailable','desktop_unavailable','native_bridge_unavailable'].includes(cause?.code))code='state_unavailable';
        else if(cause?.code==='desktop_busy')code='state_busy';
        else if(cause?.code==='desktop_call_limit')code='state_limit';
        else if(cause?.code==='desktop_timeout')code='timeout';
        else if(cause?.code==='desktop_protocol')code='invalid_response';
        if(cause?.effectState==='not_started')effectState='not_started';
      } catch (_) { /* 不回显状态值、宿主异常或路径。 */ }
      throw failure(code,method,effectState);
    };
    try {
      if(args.length!==(cas?4:2)||!['session','durable'].includes(args[0])||!keyValid(args[1]))throw failure('invalid_input',method,'not_started');
      const params={area:args[0],key:args[1]};
      if(cas) {
        if(!revisionValid(args[2]))throw failure('invalid_input',method,'not_started');
        try{params.expectedRevision=args[2];params.next=entry(args[3]);}catch(_){throw failure('invalid_input',method,'not_started');}
      }
      const checked=raw=>{
        try {
          if(!cas)return snapshot(raw);
          const f=fields(raw);
          if(!exact(f,['status','current'])||!['committed','conflict'].includes(f.status))throw new Error();
          return Object.freeze({status:f.status,current:snapshot(f.current)});
        } catch (_) {throw failure('invalid_response',method,cas?'unknown':'not_started');}
      };
      sent=true;const result=invoke(method,params);
      return asyncMode?Promise.resolve(result).then(checked).catch(rejected):checked(result);
    } catch (cause) {return rejected(cause);}
  }
  return Object.freeze({
    read:asyncMode?async(...args)=>operation('state.read',args):(...args)=>operation('state.read',args),
    compareAndSwap:asyncMode?async(...args)=>operation('state.compare_and_swap',args):(...args)=>operation('state.compare_and_swap',args),
  });
}
