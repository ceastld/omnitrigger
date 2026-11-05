// 仅做文本层面的本地盘符路径分解，不查询文件系统或 Explorer。
const fail=code=>{throw Object.assign(new Error('路径文本参数或结果无效。'),{code});};
function validText(text){
  if(typeof text!=='string'||text.length>65536||/[\x00-\x1f\x7f-\x9f]/.test(text))return false;
  for(let i=0;i<text.length;i++){
    const n=text.charCodeAt(i);
    if(n>=0xd800&&n<=0xdbff){const next=text.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))return false;}
    else if(n>=0xdc00&&n<=0xdfff)return false;
  }
  return true;
}
export function transform(paths,operation){
  if(!['name','stem','parent'].includes(operation)||!Array.isArray(paths)||paths.length<1||paths.length>128
    ||Reflect.ownKeys(paths).length!==paths.length+1)fail('invalid_input');
  const values=[];let inputUnits=0;
  for(let i=0;i<paths.length;i++){
    if(!Object.hasOwn(paths,i))fail('invalid_input');
    const path=paths[i];
    if(!validText(path)||!/^[a-z]:[\\/]/i.test(path))fail('invalid_input');
    inputUnits+=path.length;if(inputUnits>65536)fail('input_too_large');
    const normalized=path.replace(/\//g,'\\');
    const parts=normalized.slice(3).split('\\');
    if(parts.at(-1)==='')parts.pop();
    if(parts.some(part=>!part||part==='.'||part==='..'||/[<>:"|?*]/.test(part)||/[ .]$/.test(part)))fail('invalid_input');
    const root=normalized.slice(0,3);
    if(operation==='parent')values.push(parts.length<2?root:root+parts.slice(0,-1).join('\\'));
    else{
      if(parts.length===0)fail('path_name_missing');
      const name=parts.at(-1);const dot=name.lastIndexOf('.');
      values.push(operation==='stem'&&dot>0?name.slice(0,dot):name);
    }
  }
  const text=values.join('\n');
  if(text.length>65536)fail('output_too_large');
  return{text,summary:{operation,inputItems:paths.length,outputItems:values.length,inputUtf16Units:inputUnits,outputUtf16Units:text.length}};
}
