// 规则固定且不依赖 locale、文件系统或隐含宿主状态。
const fail=code=>{throw Object.assign(new Error('文本行参数或结果无效。'),{code});};
export function transform(text,operation){
  if(typeof text!=='string'||text.includes('\0')||!['deduplicate','sort'].includes(operation))fail('invalid_input');
  if(text.length>65536)fail('input_too_large');
  for(let i=0;i<text.length;i++){
    const unit=text.charCodeAt(i);
    if(unit>=0xd800&&unit<=0xdbff){const next=text.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))fail('invalid_input');}
    else if(unit>=0xdc00&&unit<=0xdfff)fail('invalid_input');
  }
  const normalized=text.replace(/\r\n|\r/g,'\n');
  const terminalNewline=normalized.endsWith('\n');
  const lines=normalized===''?[]:(terminalNewline?normalized.slice(0,-1):normalized).split('\n');
  if(lines.length>10000)fail('too_many_lines');
  const result=operation==='deduplicate'?[...new Set(lines)]:[...lines].sort((a,b)=>a<b?-1:a>b?1:0);
  const output=result.join('\n')+(terminalNewline?'\n':'');
  if(output.length>65536)fail('output_too_large');
  return{text:output,summary:{operation,inputLines:lines.length,outputLines:result.length,removedLines:lines.length-result.length,
    inputUtf16Units:text.length,outputUtf16Units:output.length,terminalNewline,changed:output!==text}};
}
