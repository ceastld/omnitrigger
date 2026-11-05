// 内存模拟复制与打开请求，不访问真实剪贴板或浏览器。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const script = readFileSync(new URL('search.js', import.meta.url), 'utf8');

test('两个脚本动作通过清单启用单实例，既有调用信封仍引用原动作契约', () => {
    const manifest = JSON.parse(readFileSync(new URL('action.json', import.meta.url), 'utf8'));
    assert.equal(manifest.schema_version, 1);
    assert.equal(manifest.id, 'examples.common-search');
    for (const [id, entry] of [['web-search', 'search.js'], ['open-or-search', 'open-or-search.js']]) {
        const action = manifest.actions.find(item => item.id === id);
        assert.ok(action, `清单缺少 ${id}`);
        assert.equal(action.single_instance, true, `${id} 必须由 ActionService 执行单实例策略`);
        assert.equal(action.runtime, 'script');
        assert.equal(action.entry, entry);
        assert.equal(action.version, 1);
    }
    for (const name of ['call.json', 'text-call.json', 'open-or-search-call.json', 'open-file-call.json']) {
        const call = JSON.parse(readFileSync(new URL(name, import.meta.url), 'utf8'));
        const action = manifest.actions.find(item => item.id === call.action_id);
        assert.ok(action, `${name} 必须指向清单中的动作`);
        assert.equal(call.package_id, manifest.id);
        assert.equal(call.action_version, action.version);
        assert.equal(Object.hasOwn(call.input, 'single_instance'), false, '单实例属于清单，不属于调用输入');
    }
});

function run(input, selected = 'a & 中\n文', failureAt = '') {
    const calls = [];
    const fault = Object.assign(new Error('宿主失败'), {code: 'target_changed', effectState: 'not_started'});
    const ctx = {input, desktop: {copySelectionText() {
        calls.push(['copy']);
        if (failureAt === 'copy') throw fault;
        return selected;
    }}, open(url) {
        calls.push(['open', url]);
        if (failureAt === 'open') throw fault;
    }};
    try {
        const result = runInNewContext(`${script}\nmain(ctx)`, {ctx});
        return {result: JSON.parse(JSON.stringify(result)), calls};
    } catch (error) {
        return {error, calls, fault};
    }
}

test('只复制新选区，URL 编码保留查询边界，结果不泄露内容', () => {
    const output = run({source: 'selection'});
    assert.deepEqual(output.calls, [['copy'], ['open', 'https://www.google.com/search?q=a%20%26%20%E4%B8%AD%0A%E6%96%87']]);
    assert.deepEqual(output.result, {submitted: true, source: 'selection', queryCodePoints: 7, truncated: false});
});

test('明确文本不读取桌面，模板与搜索词分开处理', () => {
    const output = run({source: 'text', text: 'a+b/#?%', urlTemplate: 'https://example.com/find?q=%s&lang=zh'});
    assert.deepEqual(output.calls, [['open', 'https://example.com/find?q=a%2Bb%2F%23%3F%25&lang=zh']]);
});

test('截断按 Unicode 码点，不拆开代理对，保留原始空白', () => {
    const output = run({source: 'text', text: ' 😀后续', maxCodePoints: 2});
    assert.deepEqual(output.calls, [['open', 'https://www.google.com/search?q=%20%F0%9F%98%80']]);
    assert.deepEqual(output.result, {submitted: true, source: 'text', queryCodePoints: 2, truncated: true});
    const defaults = run({source: 'text', text: '😀'.repeat(129)});
    assert.equal(defaults.result.queryCodePoints, 128);
    assert.equal(defaults.result.truncated, true);
    assert.equal(defaults.calls[0][1].split('%F0%9F%98%80').length, 129);
});

test('模板、来源、长度和上限错误均早于复制与打开', () => {
    for (const urlTemplate of ['', null, 'file:///x/%s', 'javascript:%s', 'https://%s/x',
        'https://example.com/', 'https://example.com/%s/%s', 'https://u:p@example.com/%s',
        'https://example.com/\n%s', 'https://example.com/\\%s', 'https://example.com/\0%s', 'https://example.com/\ud800/%s']) {
        const output = run({source: 'selection', urlTemplate});
        assert.equal(output.error?.code, 'invalid_input', String(urlTemplate));
        assert.deepEqual(output.calls, []);
    }
    for (const maxCodePoints of [null, true, 0, -1, 2049, 0.5, NaN, Infinity]) {
        const output = run({source: 'selection', maxCodePoints});
        assert.equal(output.error?.code, 'invalid_input');
        assert.deepEqual(output.calls, []);
    }
    for (const source of [undefined, null, 'clipboard', 'Selection']) {
        assert.deepEqual(run({source}).calls, []);
        assert.equal(run({source}).error?.code, 'invalid_input');
    }
    for (const text of [undefined, null, '', ' \n', 'bad\0text', 'a'.repeat(65537), '\ud800', 'ok\udfff']) {
        const output = run({source: 'text', text, maxCodePoints: 1});
        assert.equal(output.error?.code, 'invalid_input');
        assert.deepEqual(output.calls, []);
    }
});

test('空选区或无效 Unicode 不搜索旧剪贴板，复制副作用明确保留', () => {
    for (const selection of ['', ' \n', 'a\0b', '\ud800', 'a\udfff', 'a'.repeat(65537)]) {
        const output = run({source: 'selection'}, selection);
        assert.equal(output.error?.code, 'invalid_selection');
        assert.equal(output.error?.effectState, 'unknown');
        assert.deepEqual(output.calls, [['copy']]);
    }
});

test('复制或打开失败原样传播，不重试', () => {
    for (const [failureAt, count] of [['copy', 1], ['open', 2]]) {
        const output = run({source: 'selection'}, 'query', failureAt);
        assert.equal(output.error, output.fault);
        assert.equal(output.calls.length, count);
    }
});
