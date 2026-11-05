// 只调用内存 ctx，VM 中不提供系统 Clipboard、进程或本机输入 API。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('paste-arrow.js', import.meta.url), 'utf8');
const valid = {text: '\\Leftarrow ', pasteKey: 'ctrl-v', keyDownDelayMs: 0};
function run(input, {failureAt = '', foreground = true, mutateInput = false} = {}) {
    const calls = [];
    const fault = Object.assign(new Error('合成宿主拒绝'), {
        code: failureAt === 'delay' ? 'cancelled' : 'target_changed',
        effectState: failureAt === 'write' ? 'unknown' : 'not_started',
    });
    const invoke = (method, args, result) => {
        calls.push([method, args]);
        if (method === failureAt) throw fault;
        return result;
    };
    const ctx = {input, desktop: {
        target: () => invoke('target', [], {isForeground: foreground}),
        sendShortcut: (...args) => invoke('paste', args, {submitted: true}),
    }, clipboard: {
        writeText: text => {
            const receipt = invoke('write', [text], {written: true, sequenceNumber: 7});
            if (mutateInput) Object.assign(input, {text: '\\Rightarrow ', pasteKey: 'shift-insert', keyDownDelayMs: 999});
            return receipt;
        },
    }, delay: ms => invoke('delay', [ms], undefined)};
    try {
        const result = runInNewContext(`${source}\nmain(ctx)`, {ctx});
        return {result: JSON.parse(JSON.stringify(result)), calls: JSON.parse(JSON.stringify(calls)), fault};
    } catch (error) {
        return {error, calls: JSON.parse(JSON.stringify(calls)), fault};
    }
}

test('全部参数在目标读取和 Clipboard 写入前校验', () => {
    for (const change of [
        {text: '\\Leftarrow'}, {text: '← '}, {pasteKey: 'auto'},
        {keyDownDelayMs: undefined}, {keyDownDelayMs: -1}, {keyDownDelayMs: 1001},
        {restoreClipboard: true},
    ]) {
        const output = run({...valid, ...change});
        assert.equal(output.error?.code, 'invalid_input');
        assert.equal(output.error?.effectState, 'not_started');
        assert.deepEqual(output.calls, []);
    }
});

test('两种字面文本与粘贴键明确发送，写入后不再读取可变参数', () => {
    for (const [text, pasteKey, keys, keyDownDelayMs] of [
        ['\\Leftarrow ', 'ctrl-v', [162, 86], 0],
        ['\\Rightarrow ', 'shift-insert', [160, 45], 10],
    ]) {
        const output = run({text, pasteKey, keyDownDelayMs}, {mutateInput: true});
        assert.deepEqual(output.calls, [
            ['target', []], ['write', [text]], ['delay', [50]],
            ['paste', [keys, {keyDownDelayMs}]],
        ]);
        assert.deepEqual(output.result, {clipboardWritten: true, pasteInputSubmitted: true, pasteKey});
    }
});

test('写入后等待或粘贴被拒绝仍保留 Clipboard 效果，不恢复或重试', () => {
    for (const failureAt of ['delay', 'paste']) {
        const output = run({...valid}, {failureAt});
        assert.equal(output.error?.code, 'clipboard_written_paste_unconfirmed');
        assert.equal(output.error?.effectState, 'unknown');
        assert.match(output.error?.message, /剪贴板已经写入/);
        assert.equal(output.error?.cause, output.fault);
        assert.equal(output.error?.method, failureAt === 'delay' ? 'delay' : 'keyboard.shortcut');
        assert.deepEqual(output.calls.map(([method]) => method),
            failureAt === 'delay' ? ['target', 'write', 'delay'] : ['target', 'write', 'delay', 'paste']);
    }
});

test('目标拒绝发生在写入前，写入错误不继续粘贴且保留原始状态', () => {
    const stale = run({...valid}, {foreground: false});
    assert.equal(stale.error?.code, 'target_changed');
    assert.equal(stale.error?.effectState, 'not_started');
    assert.deepEqual(stale.calls, [['target', []]]);
    for (const failureAt of ['target', 'write']) {
        const output = run({...valid}, {failureAt});
        assert.equal(output.error, output.fault);
        assert.deepEqual(output.calls.map(([method]) => method),
            failureAt === 'target' ? ['target'] : ['target', 'write']);
    }
});
