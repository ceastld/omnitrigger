/** @param {string} [code] @returns {never} */
function fail(code='invalid_response'){throw Object.assign(Error('窗口只读快照不完整或已变化。'),{code,effectState:'not_started'});}
/** @param {unknown} value @param {readonly string[]} names @returns {Record<string,any>} */
function row(value,names){
 if(!value||typeof value!=='object'||Array.isArray(value)||![Object.prototype,null].includes(Object.getPrototypeOf(value)))fail();
 const d=Object.getOwnPropertyDescriptors(value);
 if(Reflect.ownKeys(d).length!==names.length||names.some(k=>!Object.hasOwn(d,k)||!Object.hasOwn(d[k],'value')))fail();
 return Object.fromEntries(names.map(k=>[k,d[k].value]));
}
const identityKeys=['window_handle','process_id','thread_id','process_created_file_time'];
/** @param {unknown} value @returns {import('./types.js').Identity} */
function identity(value){
 const r=row(value,identityKeys);
 for(const k of ['window_handle','process_created_file_time'])if(typeof r[k]!=='string'||!/^[1-9][0-9]*$/.test(r[k])||r[k].length>20||r[k].length===20&&r[k]>'18446744073709551615')fail();
 for(const k of ['process_id','thread_id'])if(!Number.isInteger(r[k])||r[k]<1||r[k]>4294967295)fail();
 return Object.freeze(/** @type {import('./types.js').Identity} */(r));
}
/** @param {unknown} value */
function target(value){
 const r=row(value,[...identityKeys,'expires_at_unix_ms']);
 if(!Number.isSafeInteger(r.expires_at_unix_ms)||r.expires_at_unix_ms<0)fail();
 return identity(Object.fromEntries(identityKeys.map(k=>[k,r[k]])));
}
/** @param {unknown} value @returns {import('./types.js').Rect} */
function rect(value){
 const r=row(value,['left','top','width','height']);
 if(!Object.values(r).every(n=>Number.isInteger(n)&&n>=-2147483648&&n<=2147483647)||r.width<1||r.height<1||r.left+r.width>2147483647||r.top+r.height>2147483647)fail();
 return Object.freeze(/** @type {import('./types.js').Rect} */(r));
}
/** 立即复制每个SDK回执，避免后续等待期间借来的对象变化掩盖差异。
 * @param {unknown} value @returns {import('./sdk/api.js').WindowGeometry} */
function geometry(value){
 const r=row(value,['coordinateSpace','className','outerRect','visibleFrameRect','monitorRect','workArea','primaryScreen','windowDpi','maximized']);
 const primary=row(r.primaryScreen,['width','height']);
 if(typeof r.className!=='string'||!r.className.length||r.className.length>255||r.className.includes('\0'))fail();
 for(let i=0;i<r.className.length;i++){const u=r.className.charCodeAt(i);if(u>=0xd800&&u<=0xdbff){const n=r.className.charCodeAt(++i);if(!(n>=0xdc00&&n<=0xdfff))fail();}else if(u>=0xdc00&&u<=0xdfff)fail();}
 if(r.coordinateSpace!=='physical-screen'||!Number.isInteger(r.windowDpi)||r.windowDpi<1||r.windowDpi>65535||typeof r.maximized!=='boolean'||!Object.values(primary).every(n=>Number.isInteger(n)&&n>0&&n<=2147483647))fail();
 return Object.freeze({coordinateSpace:'physical-screen',className:r.className,outerRect:rect(r.outerRect),visibleFrameRect:rect(r.visibleFrameRect),
  monitorRect:rect(r.monitorRect),workArea:rect(r.workArea),primaryScreen:Object.freeze(/** @type {{width:number;height:number}} */(primary)),windowDpi:r.windowDpi,maximized:r.maximized});
}
/** @param {unknown} value @returns {import('./sdk/api.js').PointingRootObservation} */
function pointing(value){
 const r=row(value,['sampledAt','coordinateSpace','point','inputSource','pointingWindow','rootWindow']);
 const p=row(r.point,['x','y']),s=row(r.inputSource,['kind','injected','selfInjected']);
 if(r.sampledAt!=='call-time'||r.coordinateSpace!=='physical-screen'||!Object.values(p).every(n=>Number.isInteger(n)&&n>=-2147483648&&n<=2147483647)||s.kind!=='polled-pointer'||s.injected!==null||s.selfInjected!==null)fail();
 return Object.freeze({sampledAt:'call-time',coordinateSpace:'physical-screen',point:Object.freeze(/** @type {{x:number;y:number}} */(p)),
  inputSource:Object.freeze({kind:'polled-pointer',injected:null,selfInjected:null}),pointingWindow:identity(r.pointingWindow),rootWindow:identity(r.rootWindow)});
}

export {target as decodeOriginal,geometry as decodeGeometry,pointing as decodePointing};
