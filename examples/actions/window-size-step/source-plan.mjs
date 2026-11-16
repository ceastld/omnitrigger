// 纯来源数值叶；只规划，不读取SDK，不授权写入，不宣称完成窗口大小调整。
const MIN=-2147483648,MAX=2147483647;
/** @param {string} code @returns {never} */
function fail(code){throw Object.assign(new Error('窗口大小来源数值或矩形范围无效。'),{code,effectState:'not_started'});}
/** @param {unknown} value @param {readonly string[]} names @returns {Record<string,any>} */
function row(value,names){
 if(!value||typeof value!=='object'||Array.isArray(value)||![Object.prototype,null].includes(Object.getPrototypeOf(value)))fail('invalid_input');
 const d=Object.getOwnPropertyDescriptors(value);
 if(Reflect.ownKeys(d).length!==names.length||names.some(k=>!Object.hasOwn(d,k)||!Object.hasOwn(d[k],'value')))fail('invalid_input');
 return Object.fromEntries(names.map(k=>[k,d[k].value]));
}
/** @param {unknown} n @returns {number} */
function i32(n){if(typeof n!=='number'||!Number.isInteger(n)||n<MIN||n>MAX)fail('coordinate_out_of_range');return n===0?0:n;}

/** Width/Height 为来源 Number(Double)，调节大小为 Integer；i32±10..200 精确，无半整数。
 * 两个尺寸使用各自完整表达式，先 checked Int32，左上角不变；主屏只记录，不钳制。
 * 负尺寸的原 WindowOperation 哨兵语义和零尺寸均不冒充现有 SDK 的正尺寸。
 * @param {unknown} value */
export function planSourceSizeStep(value){
 const input=row(value,['rect','primaryScreen','operation','step']),r=row(input.rect,['left','top','width','height']);
 const p=row(input.primaryScreen,['width','height']);
 for(const k of ['left','top','width','height'])r[k]=i32(r[k]);
 if(r.width<=0||r.height<=0)fail('invalid_input');
 i32(r.left+r.width);i32(r.top+r.height);
 if(!Number.isInteger(input.step)||input.step<10||input.step>200||!['increase','decrease'].includes(input.operation))fail('invalid_input');
 if(i32(p.width)<=0||i32(p.height)<=0)fail('invalid_input');
 const delta=input.operation==='increase'?input.step:-input.step;
 const requestedRect=Object.freeze({left:r.left,top:r.top,width:i32(r.width+delta),height:i32(r.height+delta)});
 /** @type {string[]} */const missing=[];
 if(requestedRect.width<0||requestedRect.height<0)missing.push('source_negative_size_semantics');
 if(requestedRect.width===0||requestedRect.height===0)missing.push('source_zero_size_semantics');
 if(requestedRect.width>65535||requestedRect.height>65535)missing.push('sdk_size_limit');
 if(requestedRect.left+requestedRect.width>MAX||requestedRect.top+requestedRect.height>MAX)missing.push('sdk_rect_edge_limit');
 return Object.freeze({kind:'source_numeric_plan',requestedRect,primaryScreen:Object.freeze({width:p.width,height:p.height}),
  conversion:'exact-binary64-integer-then-checked-int32',dimensionGaps:Object.freeze(missing),
  sdkRect:missing.length===0?requestedRect:null,submitted:false,windowChangeConfirmed:false});
}
