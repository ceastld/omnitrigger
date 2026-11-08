import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../js-engine-probe/src/text-encoder.js', import.meta.url), 'utf8');
const Native = globalThis.TextEncoder;
const native = new Native();
const context = vm.createContext({});
vm.runInContext(source, context, { timeout: 1000, filename: 'text-encoder.js' });
const Poly = vm.runInContext('TextEncoder', context);
const poly = new Poly();
const foreign = vm.createContext({});
const array = value => Array.from(value);
const plain = value => ({ read: value.read, written: value.written });
const typeError = cause => cause?.name === 'TypeError';
const report = { pureNodeOnly: true, Node: process.version, ScriptHostExecuted: false,
  cases: [], differentialEncodeCases: 0, differentialIntoCases: 0,
  oracleDifference: 'Node encodeInto拒绝非字符串；本polyfill遵循WebIDL USVString转换。可变大小目标按IDL拒绝，不以Node宽松行为为准。' };
const samples = ['', 'ASCII\0xyz', '中文测试', 'A中😀Z', '\ud800', '\udc00', '\ud800A\udc00', '\ud800\ud800\udc00', '\udbff\udfff', '\u007f\u0080\u07ff\u0800\uffff', '\ufeff'];

test('encode：字符边界、USV孤立代理、空/undefined/null与原生差分', () => {
  for (const value of [...samples, undefined, null, true, 42, 42n, { toString: () => '中\ud800😀' }]) {
    assert.deepEqual(array(poly.encode(value)), array(native.encode(value)));
    report.differentialEncodeCases++;
  }
  assert.deepEqual(array(poly.encode()), array(native.encode()));
  assert.notEqual(poly.encode(''), poly.encode(''));
  assert.equal(Object.getPrototypeOf(poly.encode('')), vm.runInContext('Uint8Array.prototype', context));
  assert.throws(() => poly.encode(Symbol('x')), typeError);
  assert.throws(() => native.encode(Symbol('x')), typeError);
  const hints = [];
  assert.deepEqual(array(poly.encode({ [Symbol.toPrimitive](hint) { hints.push(hint); return '中'; } })), array(native.encode('中')));
  assert.deepEqual(hints, ['string']);
  report.cases.push('encode-USV-and-conversion');
});

test('encodeInto：所有小缓冲区边界、偏移视图、UTF16 read与完整码点', () => {
  for (const text of samples) {
    const size = native.encode(text).length;
    for (let length = 0; length <= size + 2; length++) {
      const actual = new Uint8Array(length + 4).fill(0xa5);
      const expected = new Uint8Array(length + 4).fill(0xa5);
      const a = poly.encodeInto(text, actual.subarray(2, 2 + length));
      const b = native.encodeInto(text, expected.subarray(2, 2 + length));
      assert.deepEqual(plain(a), plain(b), JSON.stringify({ text, length }));
      assert.deepEqual(actual, expected);
      assert.deepEqual(array(actual.subarray(2, 2 + a.written)), array(native.encode(text.slice(0, a.read))));
      report.differentialIntoCases++;
    }
  }
  const d = new Uint8Array(3).fill(0xa5);
  assert.deepEqual(plain(poly.encodeInto('😀', d)), { read: 0, written: 0 });
  assert.deepEqual(array(d), [0xa5, 0xa5, 0xa5]);
  assert.deepEqual(plain(poly.encodeInto('\ud800', d)), { read: 1, written: 3 });
  report.cases.push('encodeInto-byte-boundaries-and-offsets');
});

test('确定性UTF16差分：包括成对与不成对代理，不靠少量示例掩盖状态错误', () => {
  let seed = 0x12345678;
  const next = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0);
  for (let index = 0; index < 160; index++) {
    let text = '';
    for (let count = next() % 17; count > 0; count--) text += String.fromCharCode(next() & 0xffff);
    if (index % 3 === 0) text += '\ud800😀\udc00';
    assert.deepEqual(array(poly.encode(text)), array(native.encode(text)));
    report.differentialEncodeCases++;
    for (let length = 0; length <= 18; length++) {
      const actual = new Uint8Array(length).fill(0xcd), expected = new Uint8Array(length).fill(0xcd);
      assert.deepEqual(plain(poly.encodeInto(text, actual)), plain(native.encodeInto(text, expected)));
      assert.deepEqual(actual, expected);
      report.differentialIntoCases++;
    }
  }
  report.cases.push('deterministic-UTF16-differential');
});

test('目的类型内槽：跨realm Uint8Array/Buffer有效，伪tag/其它类型/Proxy拒绝', () => {
  const destinations = [
    () => vm.runInContext('new Uint8Array(9)', foreign),
    () => Buffer.alloc(9),
    () => vm.runInContext('new (class extends Uint8Array {})(9)', foreign),
    () => { const d = new Uint8Array(9); Object.defineProperty(d, Symbol.toStringTag, { get() { throw Error('不读取伪tag'); } }); return d; },
    () => { const d = new Uint8Array(9); Object.defineProperty(d, 'length', { get() { throw Error('不读取伪length'); } }); return d; },
  ];
  for (const make of destinations) {
    const a = make(), b = make();
    assert.deepEqual(plain(poly.encodeInto('中😀', a)), plain(native.encodeInto('中😀', b)));
    assert.deepEqual(array(a), array(b));
  }
  const invalid = [undefined, null, [], {}, new ArrayBuffer(9), new DataView(new ArrayBuffer(9)),
    new Uint8ClampedArray(9), new Int8Array(9), new Uint16Array(9), new Proxy(new Uint8Array(9), {}),
    Object.create(Uint8Array.prototype), { [Symbol.toStringTag]: 'Uint8Array', length: 9 },
    Object.defineProperty(new Uint8ClampedArray(9), Symbol.toStringTag, { value: 'Uint8Array' })];
  for (const d of invalid) {
    assert.throws(() => poly.encodeInto('a', d), typeError);
    assert.throws(() => native.encodeInto('a', d), typeError);
  }
  report.cases.push('destination-internal-brand-cross-realm-and-Buffer');
});

test('实例品牌与API形状：错误接收器不可伪造，已有原生全局完全保留', () => {
  assert.equal(poly.encoding, 'utf-8');
  assert.equal(new Poly('utf-16').encoding, 'utf-8');
  assert.equal(Poly.length, 0);
  assert.equal(Poly.prototype.encode.length, 0);
  assert.equal(Poly.prototype.encodeInto.length, 2);
  assert.equal(Object.prototype.toString.call(poly), '[object TextEncoder]');
  assert.throws(() => Poly(), typeError);
  for (const receiver of [undefined, null, {}, Poly.prototype, Object.create(Poly.prototype),
    { [Symbol.toStringTag]: 'TextEncoder' }, new Proxy(poly, {})]) {
    assert.throws(() => Poly.prototype.encode.call(receiver, 'a'), typeError);
    assert.throws(() => Poly.prototype.encodeInto.call(receiver, 'a', new Uint8Array(2)), typeError);
    assert.throws(() => Object.getOwnPropertyDescriptor(Poly.prototype, 'encoding').get.call(receiver), typeError);
  }
  for (const receiver of [undefined, null, {}, Native.prototype, Object.create(Native.prototype)]) {
    assert.throws(() => Native.prototype.encode.call(receiver, 'a'), typeError);
    assert.throws(() => Native.prototype.encodeInto.call(receiver, 'a', new Uint8Array(2)), typeError);
  }
  const preserved = vm.createContext({ TextEncoder: Native });
  const before = Object.getOwnPropertyDescriptor(preserved, 'TextEncoder');
  vm.runInContext(source, preserved, { timeout: 1000 });
  assert.equal(preserved.TextEncoder, Native);
  assert.deepEqual(Object.getOwnPropertyDescriptor(preserved, 'TextEncoder'), before);
  vm.runInContext(source, context, { timeout: 1000 });
  assert.equal(vm.runInContext('TextEncoder', context), Poly);
  class Child extends Poly {}
  assert.deepEqual(array(new Child().encode('中')), array(native.encode('中')));
  report.cases.push('encoder-brand-shape-preserve-native-and-idempotence');
});

test('标准IDL转换与Node oracle边界：encodeInto非字符串、缺参、Symbol及副作用', () => {
  for (const value of [undefined, null, 42, true, { toString: () => '中\ud800' }]) {
    const a = new Uint8Array(40), b = new Uint8Array(40);
    assert.deepEqual(plain(poly.encodeInto(value, a)), plain(native.encodeInto(`${value}`, b)));
    assert.deepEqual(a, b);
    assert.throws(() => native.encodeInto(value, new Uint8Array(40)), typeError);
  }
  assert.throws(() => poly.encodeInto(), typeError);
  assert.throws(() => poly.encodeInto('a'), typeError);
  assert.throws(() => poly.encodeInto(Symbol('x'), new Uint8Array(9)), typeError);
  const failure = new Error('转换失败');
  assert.throws(() => poly.encode({ toString() { throw failure; } }), cause => cause === failure);
  assert.throws(() => poly.encodeInto({ toString() { throw failure; } }, {}), cause => cause === failure);
  const d = new Uint8Array(9).fill(0xb5);
  assert.throws(() => poly.encodeInto({ toString() { throw failure; } }, d), cause => cause === failure);
  assert.deepEqual(array(d), Array(9).fill(0xb5));
  report.cases.push('WebIDL-conversion-versus-Node-string-only-oracle');
});

test('固定共享缓冲区允许；detached零容量；可变缓冲区按IDL拒绝', () => {
  const a = new Uint8Array(new SharedArrayBuffer(9)), b = new Uint8Array(new SharedArrayBuffer(9));
  assert.deepEqual(plain(poly.encodeInto('中😀', a)), plain(native.encodeInto('中😀', b)));
  assert.deepEqual(a, b);
  const detached = new Uint8Array(3);
  structuredClone(detached.buffer, { transfer: [detached.buffer] });
  assert.deepEqual(plain(poly.encodeInto('a', detached)), plain(native.encodeInto('a', detached)));
  assert.deepEqual(plain(poly.encodeInto('', detached)), { read: 0, written: 0 });
  const resizable = new Uint8Array(new ArrayBuffer(8, { maxByteLength: 16 }));
  assert.throws(() => poly.encodeInto('a', resizable), typeError);
  const growable = new Uint8Array(new SharedArrayBuffer(8, { maxByteLength: 16 }));
  assert.throws(() => poly.encodeInto('a', growable), typeError);
  report.cases.push('shared-fixed-detached-zero-capacity-resizable-rejected');
});


