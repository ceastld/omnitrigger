import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../crates/omnitrigger-actions/runtime/host.mjs', import.meta.url), 'utf8');
const startSource = source.slice(source.indexOf('function validateProcess('), source.indexOf('\nfunction run('));

function host(fail = false) {
  const calls = [];
  let unrefs = 0;
  const context = vm.createContext({
    Buffer, LIMIT: 65536, request: { package_root: 'D:/owned-package' },
    checkCancelled() {},
    fault: (code, message) => Object.assign(new Error(message), { code }),
    spawn(program, args, options) {
      calls.push(JSON.parse(JSON.stringify({ program, args, options })));
      const child = new EventEmitter();
      child.pid = 421;
      child.unref = () => { unrefs++; };
      queueMicrotask(() => child.emit(fail ? 'error' : 'spawn', new Error('启动失败')));
      return child;
    },
  });
  vm.runInContext(`${startSource}\nglobalThis.startProcess = start;`, context);
  return { start: context.startProcess, calls, unrefs: () => unrefs };
}

test('省略隐藏选项保持独立启动默认值', async () => {
  const h = host();
  assert.equal((await h.start('owned.exe')).pid, 421);
  assert.deepEqual(h.calls, [{
    program: 'owned.exe', args: [],
    options: { cwd: 'D:/owned-package', shell: false, windowsHide: true, detached: true, stdio: 'ignore' },
  }]);
  assert.equal(h.unrefs(), 1);
});

test('显式 false 只取消隐藏请求，参数不经过 shell', async () => {
  const h = host();
  await h.start('owned.exe', ['a b', '&echo'], { cwd: 'D:/owned-work', windowsHide: false });
  assert.deepEqual(h.calls, [{
    program: 'owned.exe', args: ['a b', '&echo'],
    options: { cwd: 'D:/owned-work', shell: false, windowsHide: false, detached: true, stdio: 'ignore' },
  }]);
});

test('true 和 undefined 均保留隐藏请求', async () => {
  for (const windowsHide of [true, undefined]) {
    const h = host();
    await h.start('owned.exe', [], { windowsHide });
    assert.equal(h.calls[0].options.windowsHide, true);
  }
});

test('非布尔值和未知选项在创建进程前拒绝', () => {
  for (const options of [{ windowsHide: null }, { windowsHide: 'false' }, { windowsHide: 0 }, { windowsHide: {} }, { visible: true }]) {
    const h = host();
    assert.throws(() => h.start('owned.exe', [], options), error => error.code === 'invalid_input');
    assert.equal(h.calls.length, 0);
  }
});

test('创建失败保留结构化错误且不解除引用', async () => {
  const h = host(true);
  await assert.rejects(h.start('owned.exe'), error => error.code === 'process_start_failed');
  assert.equal(h.unrefs(), 0);
});

test('PID 在 spawn 后返回，不等待目标退出或界面事件', async () => {
  const h = host();
  assert.equal((await h.start('owned.exe', [], { windowsHide: false })).pid, 421);
  assert.equal(h.calls.length, 1);
});
