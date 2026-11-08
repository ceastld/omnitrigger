// MIT；独立 UTF-8 编码环境，无宿主调用或外部依赖。在每个引擎上下文初始化时执行一次。
(() => {
  'use strict';
  if (globalThis.TextEncoder !== undefined) return;

  const U8 = Uint8Array;
  const isView = ArrayBuffer.isView;
  const typedArrayPrototype = Object.getPrototypeOf(U8.prototype);
  const getter = (object, name) => Object.getOwnPropertyDescriptor(object, name)?.get;
  const getTag = getter(typedArrayPrototype, Symbol.toStringTag);
  const getLength = getter(typedArrayPrototype, 'length');
  const getBuffer = getter(typedArrayPrototype, 'buffer');
  const getResizable = getter(ArrayBuffer.prototype, 'resizable');
  const getGrowable = typeof SharedArrayBuffer === 'undefined' ? undefined : getter(SharedArrayBuffer.prototype, 'growable');
  const brands = new WeakSet();
  const brandAdd = WeakSet.prototype.add;
  const brandHas = WeakSet.prototype.has;
  const charCodeAt = String.prototype.charCodeAt;

  function receiver(value) {
    if (!brandHas.call(brands, value)) throw new TypeError('TextEncoder 接收器无效');
  }
  function variableBuffer(buffer, accessor) {
    if (!accessor) return false;
    try { return accessor.call(buffer); } catch { return false; }
  }
  function capacity(destination) {
    // 内建 isView 和 TypedArray 内槽 getter 均不读取对象自定义 toStringTag。
    if (!isView(destination) || getTag.call(destination) !== 'Uint8Array') {
      throw new TypeError('TextEncoder 目标须为 Uint8Array');
    }
    const buffer = getBuffer.call(destination);
    if (variableBuffer(buffer, getResizable) || variableBuffer(buffer, getGrowable)) {
      throw new TypeError('TextEncoder 目标须使用固定大小缓冲区');
    }
    return getLength.call(destination);
  }
  function transcode(text, destination, limit) {
    let read = 0;
    let written = 0;
    while (read < text.length) {
      let point = charCodeAt.call(text, read);
      let units = 1;
      if (point >= 0xd800 && point <= 0xdbff) {
        const low = charCodeAt.call(text, read + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          point = 0x10000 + (point - 0xd800) * 0x400 + low - 0xdc00;
          units = 2;
        } else point = 0xfffd;
      } else if (point >= 0xdc00 && point <= 0xdfff) point = 0xfffd;
      const size = point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      if (written + size > limit) break;
      if (destination !== null) {
        if (size === 1) destination[written] = point;
        else if (size === 2) {
          destination[written] = 0xc0 | (point >>> 6);
          destination[written + 1] = 0x80 | (point & 0x3f);
        } else if (size === 3) {
          destination[written] = 0xe0 | (point >>> 12);
          destination[written + 1] = 0x80 | ((point >>> 6) & 0x3f);
          destination[written + 2] = 0x80 | (point & 0x3f);
        } else {
          destination[written] = 0xf0 | (point >>> 18);
          destination[written + 1] = 0x80 | ((point >>> 12) & 0x3f);
          destination[written + 2] = 0x80 | ((point >>> 6) & 0x3f);
          destination[written + 3] = 0x80 | (point & 0x3f);
        }
      }
      written += size;
      read += units;
    }
    return { read, written };
  }
  class TextEncoder {
    constructor() { brandAdd.call(brands, this); }
    get encoding() { receiver(this); return 'utf-8'; }
    encode(input = '') {
      receiver(this);
      // 模板字符串使用抽象 ToString：保留 string hint，并拒绝 Symbol。
      const text = `${input}`;
      const length = transcode(text, null, Infinity).written;
      const bytes = new U8(length);
      transcode(text, bytes, length);
      return bytes;
    }
    encodeInto(source, destination) {
      receiver(this);
      if (arguments.length < 2) throw new TypeError('encodeInto 需要 source 和 destination');
      const text = `${source}`;
      return transcode(text, destination, capacity(destination));
    }
  }
  for (const name of ['encoding', 'encode', 'encodeInto']) {
    Object.defineProperty(TextEncoder.prototype, name, { enumerable: true });
  }
  Object.defineProperty(TextEncoder.prototype, Symbol.toStringTag, { value: 'TextEncoder', configurable: true });
  Object.defineProperty(globalThis, 'TextEncoder', { value: TextEncoder, writable: true, configurable: true });
})();
