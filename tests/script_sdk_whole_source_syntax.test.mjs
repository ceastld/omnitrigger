// 仅解析完整嵌入源码，不执行工厂、作者或宿主调用。
import test from 'node:test';
import {readFileSync} from 'node:fs';
import {Script} from 'node:vm';

for (const path of [
  '../tools/js-engine-probe/src/sdk.js',
  '../crates/omnitrigger-actions/runtime/desktop-api.js',
  '../crates/omnitrigger-actions/runtime/clipboard-history-api.js',
  '../crates/omnitrigger-actions/runtime/state-api.js',
]) {
  test(`完整嵌入 SDK 源码可解析：${path}`, () => {
    const file = new URL(path, import.meta.url);
    new Script(readFileSync(file, 'utf8'), {filename: file.pathname});
  });
}
