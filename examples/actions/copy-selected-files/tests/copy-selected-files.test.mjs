import assert from 'node:assert/strict';
import test from 'node:test';
import nodeEntry from '../main.mjs';
import { main as scriptEntry } from '../script.mjs';

const paths = ['C:\\合成😀.txt', 'C:\\文件夹', 'C:\\合成😀.txt'];
const receipt = { written: true, sequenceNumber: 123 };
function fixture(entry, selected = paths) {
  const calls = [];
  const signal = { aborted: false };
  const ctx = {
    signal,
    desktop: { explorerContext() { calls.push(['explorer']); return { currentDirectory: 'C:\\不使用', selectedPaths: selected }; } },
    fs: { stat(path) { calls.push(['stat', path]); return { kind: path.endsWith('文件夹') ? 'directory' : 'file' }; } },
    clipboard: { writeFiles(value) { calls.push(['write', [...value]]); return receipt; } },
  };
  return { ctx, calls, signal, run: () => entry(ctx) };
}
const entries = [['Node', nodeEntry], ['Script', scriptEntry]];

test('两入口只写入一次完整的有序 Unicode 文件列表，保留重复项', async () => {
  for (const [, entry] of entries) {
    const f = fixture(entry);
    assert.deepEqual(await f.run(), receipt);
    assert.deepEqual(f.calls, [['explorer'], ...paths.map(path => ['stat', path]), ['write', paths]]);
  }
});

test('公开旧 API 缺方法时在读取 Explorer 前拒绝，不退回文本剪贴板', async () => {
  for (const [, entry] of entries) {
    const f = fixture(entry);
    delete f.ctx.clipboard.writeFiles;
    f.ctx.clipboard.writeText = () => assert.fail('不得替代为文本');
    await assert.rejects(async () => f.run(), { code: 'api_unavailable', effectState: 'not_started' });
    assert.deepEqual(f.calls, []);
  }
});

test('空、超量或畸形选区零 stat、零写入，不回退当前文件夹', async () => {
  for (const [, entry] of entries) for (const selected of [[], Array(31).fill('C:\\a'), [null], [''], Array(1)]) {
    const f = fixture(entry, selected);
    await assert.rejects(async () => f.run(), { code: 'invalid_input', effectState: 'not_started' });
    assert.deepEqual(f.calls, [['explorer']]);
  }
});

test('三十项恰好消耗三十二次宿主 API，不提高宿主额度', async () => {
  for (const [, entry] of entries) {
    const f = fixture(entry, Array(30).fill('C:\\a'));
    await f.run();
    assert.equal(f.calls.length, 32);
    assert.equal(f.calls.filter(([kind]) => kind === 'write').length, 1);
  }
});

test('末项状态不可用或查询抛错时整体失败，不写入前面成功的部分', async () => {
  for (const [, entry] of entries) for (const throws of [false, true]) {
    const f = fixture(entry);
    let count = 0;
    f.ctx.fs.stat = () => {
      if (++count === paths.length) {
        if (throws) throw Object.assign(new Error('合成查询失败'), { code: 'fs_unavailable', effectState: 'not_started' });
        return { kind: 'other' };
      }
      return { kind: 'file' };
    };
    await assert.rejects(async () => f.run(), { code: throws ? 'fs_unavailable' : 'file_required', effectState: 'not_started' });
    assert.equal(f.calls.filter(([kind]) => kind === 'write').length, 0);
  }
});

test('Node 预先取消与逐项预检取消均不触碰剪贴板', async () => {
  for (const before of [true, false]) {
    const f = fixture(nodeEntry);
    f.signal.aborted = before;
    f.ctx.fs.stat = () => { f.signal.aborted = true; return { kind: 'file' }; };
    await assert.rejects(f.run(), { code: 'cancelled', effectState: 'not_started' });
    assert.equal(f.calls.filter(([kind]) => kind === 'write').length, 0);
    if (before) assert.equal(f.calls.length, 0);
  }
});

test('Node 写入返回后观察到取消为 unknown，绝不补发', async () => {
  const f = fixture(nodeEntry);
  let writes = 0;
  f.ctx.clipboard.writeFiles = () => { writes++; f.signal.aborted = true; return receipt; };
  await assert.rejects(f.run(), { code: 'cancelled', effectState: 'unknown', method: 'clipboard.write_files' });
  assert.equal(writes, 1);
});

test('选区在预检期间被外部修改不改变已经固定的列表', async () => {
  for (const [, entry] of entries) {
    const mutable = [...paths];
    const f = fixture(entry, mutable);
    f.ctx.fs.stat = () => { mutable[0] = 'C:\\后来'; mutable.push('C:\\额外'); return { kind: 'file' }; };
    await f.run();
    assert.deepEqual(f.calls.at(-1), ['write', paths]);
  }
});

test('写入失败原样保留 SDK 效果分类，每个入口最多调用一次', async () => {
  for (const [, entry] of entries) {
    const f = fixture(entry);
    let writes = 0;
    const failure = Object.assign(new Error('合成写入未确认'), { code: 'clipboard_failed', effectState: 'unknown' });
    f.ctx.clipboard.writeFiles = () => { writes++; throw failure; };
    await assert.rejects(async () => f.run(), error => error === failure);
    assert.equal(writes, 1);
  }
});
