import {isDotNetWhitespaceOnly} from './dotnet-whitespace.mjs';
/** 按完整行清理空行。
 * CRLF/CR/LF按最长匹配拆分；空字符串移除，空白行原样保留；固定CRLF连接。
 * @param {string} text
 * @returns {string[]}
 */
function nonEmptyLines(text) {
  if(typeof text!=='string') throw new TypeError('text must be a string');
  const lines=[];
  let start=0;
  for(let i=0;i<text.length;i++) {
    if(text[i]!=='\r'&&text[i]!=='\n') continue;
    if(i>start) lines.push(text.slice(start,i));
    if(text[i]==='\r'&&text[i+1]==='\n') i++;
    start=i+1;
  }
  if(start<text.length) lines.push(text.slice(start));
  return lines;
}

/** @param {string} text @returns {string} */
export function removeEmptyLines(text) { return nonEmptyLines(text).join('\r\n'); }
/** 只过滤整行空白，不trim保留行。
 * @param {string} text @returns {string}
 */
export function removeWhiteSpaceLines(text) {
  return nonEmptyLines(text).filter(line=>!isDotNetWhitespaceOnly(line)).join('\r\n');
}
