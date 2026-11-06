// 按 .NET Char.IsWhiteSpace 的UTF16字符集合判断整行空白。
// 不用JS trim/\s：U+0085应移除，U+FEFF应保留。
export const whitespaceCodeUnits=Object.freeze([9, 10, 11, 12, 13, 32, 133, 160, 5760, 8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199, 8200, 8201, 8202, 8232, 8233, 8239, 8287, 12288]);
const whitespace=new Set(whitespaceCodeUnits);
/** @param {string} text @returns {boolean} */
export function isDotNetWhitespaceOnly(text) {
  for(let i=0;i<text.length;i++) if(!whitespace.has(text.charCodeAt(i))) return false;
  return true;
}
