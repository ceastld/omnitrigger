// 全部路径、选区和元数据均为合成值；不触碰真实文件或剪贴板，不打开窗口。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';

const script = readFileSync(new URL('open-or-search.js', import.meta.url), 'utf8');
function run(input, options = {}) {
    const calls = [];
    const ctx = {
        input,
        desktop: {copySelectionText() {
            calls.push(['copy']);
            if (options.copyError) throw options.copyError;
            return options.selection ?? '合成选中文本';
        }},
        fs: options.noStat ? {} : {stat(path) {
            calls.push(['stat', path]);
            if (options.statError) throw options.statError;
            return 'metadata' in options ? options.metadata : {kind: 'file'};
        }},
        open(target) {
            calls.push(['open', target]);
            if (options.openError) throw options.openError;
        },
    };
    try {
        const result = runInNewContext(`${script}\nmain(ctx)`, {ctx});
        return {result: JSON.parse(JSON.stringify(result)), calls};
    } catch (error) {
        return {error, calls};
    }
}
const input = text => ({source: 'text', text});
const fault = code => Object.assign(new Error('合成宿主错误'), {code, method: 'fs.stat', effectState: 'not_started'});

test('普通文本不调用 stat，保留网页搜索编码、原始空白与码点截断', () => {
    const output = run({...input(' 😀后续&'), maxCodePoints: 2, urlTemplate: 'https://example.com/find?q=%s'});
    assert.deepEqual(output.calls, [['open', 'https://example.com/find?q=%20%F0%9F%98%80']]);
    assert.deepEqual(output.result, {submitted: true, source: 'text', operation: 'search', queryCodePoints: 2, truncated: true});
    const defaults = run(input('a & 中\n文'));
    assert.deepEqual(defaults.calls, [['open', 'https://www.google.com/search?q=a%20%26%20%E4%B8%AD%0A%E6%96%87']]);
    assert.equal(defaults.result.operation, 'search');
    const supplementary = run(input('😀'.repeat(129)));
    assert.equal(supplementary.result.queryCodePoints, 128);
    assert.equal(supplementary.result.truncated, true);
    assert.equal(supplementary.calls[0][1], 'https://www.google.com/search?q=' + '%F0%9F%98%80'.repeat(128));
});

test('普通文本即使无 stat 接口也可搜索，不直接打开输入中的网页 URL', () => {
    const output = run(input('https://example.com/a?q=合成'), {noStat: true});
    assert.deepEqual(output.calls, [['open', 'https://www.google.com/search?q=https%3A%2F%2Fexample.com%2Fa%3Fq%3D%E5%90%88%E6%88%90']]);
});

test('本地普通文件先 stat，再原样打开路径，不编码或截断', () => {
    for (const path of ['D:\\SyntheticExamples\\合成 文件😀.txt', 'C:/SyntheticExamples/file & query.txt', 'c:\\SyntheticExamples\\note.txt']) {
        const output = run({...input(path), maxCodePoints: 1});
        assert.deepEqual(output.calls, [['stat', path], ['open', path]]);
        assert.deepEqual(output.result, {submitted: true, source: 'text', operation: 'open-file'});
        assert.ok(!JSON.stringify(output.result).includes(path));
    }
});

test('明确目录或不存在项继续原有搜索，保留模板、原值编码与元数据类型', () => {
    for (const kind of ['directory', 'missing']) {
        const path = 'D:\\SyntheticExamples\\entry';
        const output = run({...input(path), urlTemplate: 'https://example.com/find?q=%s'}, {metadata: {kind}});
        assert.deepEqual(output.calls, [['stat', path], ['open', 'https://example.com/find?q=D%3A%5CSyntheticExamples%5Centry']]);
        assert.deepEqual(output.result, {submitted: true, source: 'text', operation: 'search',
            queryCodePoints: Array.from(path).length, truncated: false, pathKind: kind});
        assert.ok(!JSON.stringify(output.result).includes(path));
        const truncated = run({...input(path), maxCodePoints: 2}, {metadata: {kind}});
        assert.deepEqual(truncated.calls, [['stat', path], ['open', 'https://www.google.com/search?q=D%3A']]);
        assert.equal(truncated.result.truncated, true);
    }
});

test('other 明确拒绝，不搜索或打开', () => {
    const path = 'D:\\SyntheticExamples\\entry';
    const output = run(input(path), {metadata: {kind: 'other'}});
    assert.equal(output.error?.code, 'unsupported_path_type');
    assert.equal(output.error?.method, 'fs.stat');
    assert.equal(output.error?.effectState, 'not_started');
    assert.ok(!output.error.message.includes(path));
    assert.deepEqual(output.calls, [['stat', path]]);
});

test('UNC、设备、drive-relative、根相对和明显相对路径明确拒绝', () => {
    for (const path of [
        '\\\\server\\share\\file.txt', '//server/share/file.txt', '\\\\?\\C:\\file.txt', '\\\\.\\PhysicalDrive0',
        'C:file.txt', 'C:', '1:\\file.txt', 'AB:\\file.txt', '12:/file.txt', '\\file.txt', '/file.txt', './file.txt', '..\\file.txt',
        'relative\\file.txt', 'relative folder\\file.txt', '搜索正则 \\d', '~/file.txt', '%USERPROFILE%\\file.txt', '$env:USERPROFILE\\file.txt',
        '${HOME}/file.txt', 'file:///C:/file.txt', 'FILE://server/share/file.txt',
        '"D:\\SyntheticExamples\\file.txt"', "'D:/SyntheticExamples/file.txt'",
        ' D:\\SyntheticExamples\\file.txt', 'D:\\SyntheticExamples\\file.txt ',
    ]) {
        const output = run(input(path));
        assert.equal(output.error?.code, 'invalid_input', path);
        assert.deepEqual(output.calls, [], path);
        assert.ok(!output.error.message.includes(path));
    }
});

test('stat 路径校验错误原样传播，不能用查询截断或格式改写绕过', () => {
    for (const path of [
        'C:\\SyntheticExamples\\..\\file.txt', 'C:\\SyntheticExamples\\bad?.txt',
        'C:\\SyntheticExamples\\file.txt:stream', 'C:\\SyntheticExamples\\trailing. ',
        'C:\\SyntheticExamples\\CON.txt', 'C:\\SyntheticExamples\\' + 'x'.repeat(32768),
    ]) {
        const error = fault('invalid_input');
        const output = run({...input(path), maxCodePoints: 1}, {statError: error});
        // 外围空白由动作拒绝，其他词法错误由真实 SDK 和平台继续复验。
        if (path.endsWith(' ')) {
            assert.equal(output.error.code, 'invalid_input');
            assert.deepEqual(output.calls, []);
        } else {
            assert.equal(output.error, error);
            assert.deepEqual(output.calls, [['stat', path]]);
        }
    }
});

test('云端、映射盘、拒绝访问、取消、超时与宿主错误均无搜索回退', () => {
    for (const [code, path] of [
        ['path_unavailable', 'C:\\OneDriveSynthetic\\file.txt'],
        ['path_unavailable', 'D:\\SyntheticExamples\\placeholder.txt'],
        ['path_not_local', 'Z:\\SyntheticExamples\\mapped.txt'],
        ['access_denied', 'D:\\SyntheticExamples\\denied.txt'],
        ['cancelled', 'D:\\SyntheticExamples\\cancelled.txt'],
        ['timeout', 'D:\\SyntheticExamples\\timeout.txt'],
        ['fs_error', 'D:\\SyntheticExamples\\fault.txt'],
        ['fs_unavailable', 'D:\\SyntheticExamples\\unavailable.txt'],
        ['fs_busy', 'D:\\SyntheticExamples\\busy.txt'],
    ]) {
        const error = fault(code);
        const output = run(input(path), {statError: error});
        assert.equal(output.error, error);
        assert.deepEqual(output.calls, [['stat', path]]);
    }
});

test('缺失 stat 或畸形回执不当成 missing，更不搜索', () => {
    const path = 'D:\\SyntheticExamples\\file.txt';
    const unavailable = run(input(path), {noStat: true});
    assert.equal(unavailable.error?.code, 'fs_unavailable');
    assert.deepEqual(unavailable.calls, []);
    for (const metadata of [undefined, null, 'file', {}, {kind: 'unknown'}, {kind: null}]) {
        const output = run(input(path), {metadata});
        assert.equal(output.error?.code, 'invalid_response');
        assert.deepEqual(output.calls, [['stat', path]]);
    }
});

test('文件打开失败保留宿主错误，不重试、不回退搜索', () => {
    const path = 'D:\\SyntheticExamples\\file.txt';
    const error = Object.assign(new Error('合成打开失败'), {code: 'open_failed', effectState: 'unknown'});
    const output = run(input(path), {openError: error});
    assert.equal(output.error, error);
    assert.deepEqual(output.calls, [['stat', path], ['open', path]]);
});

test('选区文件只复制一次，保留新选区和后续元数据错误', () => {
    const selection = 'D:\\SyntheticExamples\\selected.txt';
    const success = run({source: 'selection'}, {selection});
    assert.deepEqual(success.calls, [['copy'], ['stat', selection], ['open', selection]]);
    assert.deepEqual(success.result, {submitted: true, source: 'selection', operation: 'open-file'});
    const error = fault('cancelled');
    const failed = run({source: 'selection'}, {selection, statError: error});
    assert.equal(failed.error, error);
    assert.deepEqual(failed.calls, [['copy'], ['stat', selection]]);
});

test('选区文本沿用搜索语义，复制失败或空选区不使用旧剪贴板', () => {
    const success = run({source: 'selection'}, {selection: '合成 & 词'});
    assert.deepEqual(success.calls, [['copy'], ['open', 'https://www.google.com/search?q=%E5%90%88%E6%88%90%20%26%20%E8%AF%8D']]);
    const error = fault('target_changed');
    const failed = run({source: 'selection'}, {copyError: error});
    assert.equal(failed.error, error);
    assert.deepEqual(failed.calls, [['copy']]);
    for (const selection of ['', ' \n', 'bad\0text', '\ud800', 'ok\udfff', 'x'.repeat(65537)]) {
        const output = run({source: 'selection'}, {selection});
        assert.equal(output.error?.code, 'invalid_selection');
        assert.equal(output.error?.effectState, 'unknown');
        assert.deepEqual(output.calls, [['copy']]);
    }
});

test('输入与模板校验早于复制、stat 和打开；完整 Unicode 校验早于截断', () => {
    for (const bad of [
        {source: 'clipboard'}, {source: 'text'}, {...input(''), maxCodePoints: 1},
        {...input('x'), maxCodePoints: 0}, {...input('x'), maxCodePoints: 2049},
        {...input('ok\udfff'), maxCodePoints: 1}, input('D:\\SyntheticExamples\\\ud800.txt'),
        {...input('x'), urlTemplate: 'javascript:%s'}, {...input('x'), urlTemplate: 'https://example.com/%s/%s'},
        {source: 'selection', urlTemplate: 'https://example.com/\ud800/%s'},
        {source: 'selection', maxCodePoints: null},
    ]) {
        const output = run(bad);
        assert.equal(output.error?.code, 'invalid_input');
        assert.deepEqual(output.calls, []);
    }
});

test('通用调用清单及旧动作契约保持明确，新增轻量入口参与类型检查', () => {
    const manifest = JSON.parse(readFileSync(new URL('action.json', import.meta.url), 'utf8'));
    assert.deepEqual(manifest.actions.find(action => action.id === 'web-search'), {
        id: 'web-search', title: '搜索选中文本或明确文本', version: 1, runtime: 'script', entry: 'search.js',
        single_instance: true,
    });
    const action = manifest.actions.find(action => action.id === 'open-or-search');
    assert.equal(action.runtime, 'script');
    assert.equal(action.version, 1);
    assert.equal(action.entry, 'open-or-search.js');
    assert.equal(action.single_instance, true);
    for (const filename of ['open-or-search-call.json', 'open-file-call.json']) {
        const call = JSON.parse(readFileSync(new URL(filename, import.meta.url), 'utf8'));
        assert.equal(call.package_id, manifest.id);
        assert.equal(call.action_id, action.id);
        assert.equal(call.action_version, action.version);
        assert.equal(call.input.source, 'text');
    }
    const config = JSON.parse(readFileSync(new URL('jsconfig.json', import.meta.url), 'utf8'));
    assert.ok(config.files.includes('open-or-search.js'));
});
