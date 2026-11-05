// 独立编写的有界 JSON 语法/排版器；数值与字符串字面量保持原 token，不转成 JS number。
/** @typedef {import('./json-types').JsonErrorCode} JsonErrorCode */
/** @param {JsonErrorCode} code @returns {never} */
function fault(code) { throw Object.assign(new Error('JSON 文本不符合本配方契约。'), {code}); }
/** @param {string} character */
const digit = character => character >= '0' && character <= '9';

/** @param {string} source @returns {import('./json-types').JsonAnalysis} */
export function analyze(source) {
  if (typeof source !== 'string' || source.length > 65536 || source.includes('\0')) fault('invalid_input');
  for (let index = 0; index < source.length; index++) {
    const unit = source.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = source.charCodeAt(++index);
      if (!(next >= 0xdc00 && next <= 0xdfff)) fault('invalid_input');
    } else if (unit >= 0xdc00 && unit <= 0xdfff) fault('invalid_input');
  }
  let position = 0;
  /** @type {string[]} */
  const tokens = [];
  /** @type {Omit<import('./json-types').JsonSummary,'rootKind'|'inputUtf16Units'>&{rootKind:import('./json-types').JsonKind|''}} */
  const summary = {rootKind:'',nodeCount:0,keyCount:0,stringCount:0,numberCount:0,maxDepth:0};
  const space = () => { while ([' ', '\r', '\n', '\t'].includes(source[position])) position++; };
  /** @param {string} value */
  const punctuation = value => { tokens.push(value); position++; };
  /** @overload @param {true} key @returns {string} */
  /** @overload @param {false} [key] @returns {null} */
  /** @param {boolean} [key] @returns {string|null} */
  function stringToken(key = false) {
    const start = position++;
    while (position < source.length) {
      const character = source[position++];
      if (character === '"') {
        const raw = source.slice(start, position);
        tokens.push(raw);
        return key ? /** @type {string} */ (JSON.parse(raw)) : null;
      }
      if (character.charCodeAt(0) < 32) fault('invalid_json');
      if (character === '\\') {
        const escape = source[position++];
        if (escape === 'u') {
          if (!/^[0-9a-fA-F]{4}$/.test(source.slice(position, position + 4))) fault('invalid_json');
          position += 4;
        } else if (!['"','\\','/','b','f','n','r','t'].includes(escape)) fault('invalid_json');
      }
    }
    fault('invalid_json');
  }
  function numberToken() {
    const start = position;
    if (source[position] === '-') position++;
    if (source[position] === '0') position++;
    else {
      if (!(source[position] >= '1' && source[position] <= '9')) fault('invalid_json');
      while (digit(source[position])) position++;
    }
    if (source[position] === '.') {
      position++;
      if (!digit(source[position])) fault('invalid_json');
      while (digit(source[position])) position++;
    }
    if (source[position] === 'e' || source[position] === 'E') {
      position++;
      if (source[position] === '+' || source[position] === '-') position++;
      if (!digit(source[position])) fault('invalid_json');
      while (digit(source[position])) position++;
    }
    tokens.push(source.slice(start, position));
  }
  /** @param {number} depth @returns {void} */
  function value(depth) {
    space();
    if (++summary.nodeCount > 10000) fault('json_limit');
    const character = source[position];
    const kind = character === '{' ? 'object' : character === '[' ? 'array' : character === '"' ? 'string'
      : character === 't' || character === 'f' ? 'boolean' : character === 'n' ? 'null' : 'number';
    if (!summary.rootKind) summary.rootKind = kind;
    if (kind === 'object' || kind === 'array') {
      if (depth >= 32) fault('json_limit');
      summary.maxDepth = Math.max(summary.maxDepth, depth + 1);
      const object = kind === 'object';
      const close = object ? '}' : ']';
      /** @type {Set<string>} */
      const names = new Set();
      punctuation(character);
      space();
      if (source[position] === close) { punctuation(close); return; }
      while (true) {
        if (object) {
          if (source[position] !== '"') fault('invalid_json');
          const key = stringToken(true);
          if (names.has(key)) fault('json_duplicate_key');
          names.add(key);
          summary.keyCount++;
          space();
          if (source[position] !== ':') fault('invalid_json');
          punctuation(':');
        }
        value(depth + 1);
        space();
        if (source[position] === close) { punctuation(close); return; }
        if (source[position] !== ',') fault('invalid_json');
        punctuation(',');
        space();
      }
    }
    if (kind === 'string') { summary.stringCount++; stringToken(); return; }
    if (kind === 'boolean' || kind === 'null') {
      const literal = character === 't' ? 'true' : character === 'f' ? 'false' : 'null';
      if (!source.startsWith(literal, position)) fault('invalid_json');
      position += literal.length;
      tokens.push(literal);
      return;
    }
    summary.numberCount++;
    numberToken();
  }
  value(0);
  space();
  if (position !== source.length) fault('invalid_json');
  return {tokens,summary:/** @type {import('./json-types').JsonSummary} */ ({...summary,inputUtf16Units:source.length})};
}

/** @param {readonly string[]} tokens @param {unknown} style @returns {string} */
export function format(tokens, style) {
  if (!['compact','pretty'].includes(/** @type {string} */ (style))) fault('invalid_input');
  if (style === 'compact') return tokens.join('');
  /** @type {string[]} */
  const parts = [];
  let units = 0;
  let depth = 0;
  /** @param {string} value */
  const append = value => {
    units += value.length;
    if (units > 65536) fault('json_output_too_large');
    parts.push(value);
  };
  const newline = () => append('\n' + '  '.repeat(depth));
  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if (token === '{' || token === '[') {
      append(token);
      depth++;
      if (!['}',']'].includes(tokens[index + 1])) newline();
    } else if (token === '}' || token === ']') {
      depth--;
      if (!['{','['].includes(tokens[index - 1])) newline();
      append(token);
    } else if (token === ',') { append(','); newline(); }
    else if (token === ':') append(': ');
    else append(token);
  }
  return parts.join('');
}
