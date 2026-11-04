// 仅运行 JS 与内存宿主，绝不调用本机输入 API。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const scripts = Object.fromEntries(['close', 'window'].map(name => [name,
    readFileSync(new URL(`${name}.js`, import.meta.url), 'utf8')]));
function run(name, input, processName = 'Code.exe', failureAt = '') {
    const calls = [];
    const fault = Object.assign(new Error('目标已变化'), {
        code: 'target_changed', effectState: 'not_started', method: failureAt,
    });
    const invoke = (method, args, result) => {
        calls.push([method, args]);
        if (method === failureAt) throw fault;
        return result;
    };
    const ctx = {input, desktop: {
        target: () => invoke('target', [], {processName}),
        sendKeySequence: (...args) => invoke('sequence', args, {submitted: true}),
        windowAction: op => invoke('window', [op], {submitted: true}),
    }};
    try {
        const result = runInNewContext(`${scripts[name]}\nmain(ctx)`, {ctx});
        return {result: JSON.parse(JSON.stringify(result)), calls: JSON.parse(JSON.stringify(calls))};
    } catch (error) {
        return {error, calls: JSON.parse(JSON.stringify(calls)), fault};
    }
}

test('关闭分支只发送一次，区分进程名大小写，去掉目标扩展名', () => {
    for (const [name, operation, modifiers, keys] of [
        ['Acrobat.exe', 'ctrl_w', [162], [87]],
        ['Acrobat.EXE', 'ctrl_w', [162], [87]],
        ['acrobat.exe', 'alt_f4', [164], [115]],
        ['chrome.exe', 'alt_f4', [164], [115]],
    ]) {
        const output = run('close', {ctrlWProcesses: ['Acrobat']}, name);
        assert.deepEqual(output.result, {processName: name, operation, submitted: true});
        assert.deepEqual(output.calls, [['target', []], ['sequence', [modifiers, keys, {keyDownDelayMs: 0}]]]);
    }
    assert.equal(run('close', {ctrlWProcesses: []}, 'Acrobat').result.operation, 'alt_f4');
});

test('参数错误先于任何目标读取或输入提交', () => {
    for (const ctrlWProcesses of [undefined, null, 'Acrobat', [1], [''], [' Acro'], ['Acro '],
        ['C:\\Acrobat'], ['Acro/name'], ['Acro,name'], ['Acrobat.exe'], ['bad\0name'], Array(65).fill('name')]) {
        const output = run('close', {ctrlWProcesses});
        assert.equal(output.error?.code, 'invalid_input');
        assert.deepEqual(output.calls, []);
    }
    for (const keyDownDelayMs of [null, false, -1, 1001, 0.5, '10', Infinity, NaN]) {
        const output = run('close', {ctrlWProcesses: ['Acrobat'], keyDownDelayMs});
        assert.equal(output.error?.effectState, 'not_started');
        assert.deepEqual(output.calls, []);
    }
});

test('默认进程延迟与显式零值保留', () => {
    for (const processName of ['wps.exe', 'ET.EXE', 'wpp', 'wpspdf.exe', 'mstsc']) {
        assert.equal(run('close', {ctrlWProcesses: []}, processName).calls[1][1][2].keyDownDelayMs, 10);
        assert.equal(run('close', {ctrlWProcesses: [], keyDownDelayMs: 0}, processName).calls[1][1][2].keyDownDelayMs, 0);
    }
    assert.equal(run('close', {ctrlWProcesses: [], keyDownDelayMs: 1000}).calls[1][1][2].keyDownDelayMs, 1000);
});

test('目标读取或输入失败原样传播，没有备用分支或重试', () => {
    for (const [failureAt, count] of [['target', 1], ['sequence', 2]]) {
        const output = run('close', {ctrlWProcesses: ['Acrobat']}, 'Acrobat.exe', failureAt);
        assert.equal(output.error, output.fault);
        assert.equal(output.calls.length, count);
    }
});

test('窗口操作只提交显式支持的操作，保留原始宿主错误', () => {
    for (const operation of ['toggle_maximize', 'minimize', 'close']) {
        const output = run('window', {operation});
        assert.deepEqual(output.calls, [['window', [operation]]]);
        assert.deepEqual(output.result, {operation, submitted: true});
    }
    for (const operation of [undefined, null, false, 'maximize', 'Close', {}, []]) {
        const output = run('window', {operation});
        assert.equal(output.error?.code, 'invalid_input');
        assert.deepEqual(output.calls, []);
    }
    const output = run('window', {operation: 'close'}, 'Code', 'window');
    assert.equal(output.error, output.fault);
    assert.equal(output.calls.length, 1);
});
