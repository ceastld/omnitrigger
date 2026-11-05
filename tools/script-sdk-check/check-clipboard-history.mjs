// 独立 CLI 持久化验收。仅使用本次 mkdtemp 合成工程，不启动 UI、监听或系统剪贴板。
import {spawn, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, unlinkSync, writeFileSync} from 'node:fs';
import {dirname, isAbsolute, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const methods = ['history.list', 'history.get', 'history.append', 'history.remove'];
const errorCodes = new Set(['invalid_input', 'history_item_missing', 'history_busy', 'history_corrupt', 'history_conflict',
  'history_unavailable', 'history_id_exhausted', 'history_revision_exhausted', 'invalid_response', 'cancelled', 'timeout']);

class CheckFailure extends Error {
  constructor(stage, detail = {}) { super(stage); this.stage = stage; this.detail = detail; }
}
function requireCheck(condition, stage, detail) { if (!condition) throw new CheckFailure(stage, detail); }
function within(parent, child) {
  const name = relative(parent, child);
  return name !== '' && name !== '..' && !name.startsWith('../') && !name.startsWith('..\\') && !isAbsolute(name);
}
function ancestors(path) {
  const result = [resolve(path)];
  while (dirname(result[0]) !== result[0]) result.unshift(dirname(result[0]));
  return result;
}

// 一次同步文件阶段只启动一个 PowerShell，批量检查明确最小路径的去重祖先。
// 作用域不跨 App 启动或 await；每次 App 调用前后重新检查属性。
let activeMetadata = null;
let metadataProcesses = 0;
export function classifyMetadataCheck(checked, expectedCount) {
  const stderr = typeof checked.stderr === 'string' ? checked.stderr : '';
  const base = {exitCode:Number.isInteger(checked.status) ? checked.status : null,
    signal:['SIGTERM','SIGKILL','SIGABRT'].includes(checked.signal) ? checked.signal : null,
    errorCode:['ETIMEDOUT','ENOENT','EACCES','EPERM','ENOBUFS'].includes(checked.error?.code) ? checked.error.code : checked.error ? 'other' : null,
    stderrSummary:{present:stderr.length > 0,bytes:Buffer.byteLength(stderr),category:stderr.length ? 'suppressed_powershell_stderr' : 'empty'}};
  if (checked.error) return {...base,classification:checked.error.code === 'ETIMEDOUT' ? 'metadata_timeout' : checked.error.code === 'ENOBUFS' ? 'metadata_output_limit' : 'metadata_spawn_failed'};
  if (![0,7,8].includes(checked.status)) return {...base,classification:'metadata_command_failed'};
  let message;
  try { message = JSON.parse(checked.stdout); } catch { return {...base,classification:'metadata_protocol_failed'}; }
  if (checked.status === 7 && message?.classification === 'attributes_rejected') {
    return {...base,classification:'unsafe_metadata_attributes',pathIndex:Number.isInteger(message.pathIndex) && message.pathIndex >= 0 && message.pathIndex < expectedCount ? message.pathIndex : null,
      blockedAttributes:Number.isInteger(message.blockedAttributes) ? message.blockedAttributes & 0x441400 : null};
  }
  if (checked.status === 8 && message?.classification === 'metadata_error') return {...base,classification:'metadata_query_failed',
    pathIndex:Number.isInteger(message.pathIndex) && message.pathIndex >= 0 && message.pathIndex < expectedCount ? message.pathIndex : null,
    exceptionType:['UnauthorizedAccessException','IOException','ItemNotFoundException','RuntimeException','ArgumentException'].includes(message.exceptionType) ? message.exceptionType : 'other'};
  if (checked.status === 0 && !checked.signal && message?.classification === 'checked' && message.checkedCount === expectedCount) return {...base,classification:'checked'};
  return {...base,classification:'metadata_command_failed'};
}
function metadataBatch(targets) {
  const paths = [...new Set(targets.flatMap(ancestors))];
  requireCheck(paths.every(value => {
    const name = value.slice(dirname(value).length).replace(/^[\\/]/, '').toLowerCase();
    return !name.startsWith('onedrive') && name !== 'dropbox' && !name.startsWith('icloud');
  }), 'unsafe_cloud_path');
  const pending = paths.filter(path => !activeMetadata?.has(path));
  if (process.platform === 'win32' && pending.length) {
    const command = `$ErrorActionPreference='Stop'; $index=-1; try { $paths=ConvertFrom-Json $env:OMNITRIGGER_HISTORY_METADATA_PATHS; if($paths -isnot [System.Array]) { $paths=@($paths) }; foreach($p in $paths) { $index++; if(Test-Path -LiteralPath $p) { $item=Get-Item -LiteralPath $p -Force; $blocked=([long]$item.Attributes -band 0x441400); if($blocked -ne 0) { @{classification='attributes_rejected';pathIndex=$index;blockedAttributes=$blocked}|ConvertTo-Json -Compress; exit 7 } } }; @{classification='checked';checkedCount=$paths.Count}|ConvertTo-Json -Compress; exit 0 } catch { @{classification='metadata_error';pathIndex=$index;exceptionType=$_.Exception.GetType().Name}|ConvertTo-Json -Compress; exit 8 }`;
    const began = Date.now();
    metadataProcesses++;
    const checked = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(command, 'utf16le').toString('base64')], {
      windowsHide:true, encoding:'utf8', timeout:15000, maxBuffer:4096,
      env:{...process.env, OMNITRIGGER_HISTORY_METADATA_PATHS:JSON.stringify(pending)},
    });
    const detail = {...classifyMetadataCheck(checked,pending.length),checkedPaths:pending.length,elapsedMs:Date.now()-began};
    requireCheck(detail.classification === 'checked',detail.classification,{metadata:detail});
  }
  return paths;
}
function withMetadata(targets, operation) {
  const previous = activeMetadata;
  const checked = metadataBatch(targets);
  activeMetadata = new Set([...(previous ?? []),...checked]);
  try {
    const result = operation();
    requireCheck(!result || typeof result.then !== 'function','metadata_scope_must_be_synchronous');
    return result;
  } finally { activeMetadata = previous; }
}
function ordinary(path, {missing = false, kind} = {}) {
  const paths = ancestors(path);
  metadataBatch([path]);
  let found;
  for (const current of paths) {
    try {
      const metadata = lstatSync(current);
      requireCheck(!metadata.isSymbolicLink(), 'unsafe_link');
      if (current === resolve(path)) found = metadata;
      else requireCheck(metadata.isDirectory(), 'unsafe_parent');
    } catch (error) {
      if (error?.code === 'ENOENT' && missing) continue;
      if (error instanceof CheckFailure) throw error;
      throw new CheckFailure('metadata_unavailable');
    }
  }
  requireCheck(found || missing, 'path_missing');
  if (found && kind) requireCheck(kind === 'file' ? found.isFile() : found.isDirectory(), 'wrong_path_kind');
  return found;
}
export function verifyOrdinaryPaths(paths) {
  requireCheck(Array.isArray(paths) && paths.length > 0 && paths.length <= 32 && paths.every(path => typeof path === 'string' && isAbsolute(path)), 'metadata_paths_invalid');
  const before = metadataProcesses;
  return withMetadata(paths, () => {
    for (const path of paths) ordinary(path,{missing:true});
    return {passed:true,requestedPaths:paths.length,distinctAncestorPaths:new Set(paths.flatMap(ancestors)).size,metadataProcesses:metadataProcesses-before,appStarted:false};
  });
}

function owned(root, path, options = {}) {
  requireCheck(within(root, resolve(path)) || resolve(path) === root && options.kind === 'directory', 'outside_owned_directory');
  return withMetadata([root,path], () => {
    ordinary(root, {kind:'directory'});
    ordinary(path, {missing:true, ...options});
    requireCheck(realpathSync(root) === root, 'owned_directory_alias');
    return path;
  });
}
function put(root, path, data, fresh = false) {
  return withMetadata([root,dirname(path),path], () => {
    owned(root, dirname(path), {kind:'directory'});
    owned(root, path, {missing:true, kind:'file'});
    writeFileSync(path, data, {flag:fresh ? 'wx' : 'w'});
  });
}
function read(root, path) { return withMetadata([root,path], () => { owned(root, path, {missing:false, kind:'file'}); return readFileSync(path); }); }
function digest(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function diagnostics(error) {
  const assertion = /\bhistory_check:([a-z_]{1,48})\b/.exec(error?.message ?? '');
  return {code:errorCodes.has(error?.code) ? error.code : 'unrecognized',
    method:methods.includes(error?.method) ? error.method : 'unrecognized',
    effectState:['not_started','unknown'].includes(error?.effect_state ?? error?.effectState) ? error.effect_state ?? error.effectState : 'unrecognized',
    ...(assertion ? {assertion:assertion[1]} : {})};
}

/** 两层真实本地模块依赖；长文本仅在运行时生成，不进入 ActionCall。 */
export function fixtureSources(node) {
  const words = String.raw`
export function sample(kind, index = 0) {
  if (kind === 'empty') return '';
  if (kind === 'unicode') return 'x'.repeat(63) + '😀合成正文_' + '🦀'.repeat(16) + '\r\n尾部';
  if (kind === 'large') return String(index).padStart(2, '0') + '\u0001'.repeat(65534);
  if (kind === 'count') return '合成正文_条目_' + index;
  if (kind === 'concurrent') return '合成正文_并发_' + index;
  return '  合成正文_持久化_🦀\r\n空白保留  ';
}
`;
  let checks = String.raw`
import {sample} from './text.mjs';
function check(ok, name) { if (!ok) throw new Error('history_check:' + name); }
function exact(value, names) { return value && typeof value === 'object' && Object.keys(value).length === names.length && names.every(name => Object.prototype.hasOwnProperty.call(value, name)); }
function validSummary(item) {
  return exact(item, ['id','preview','utf16Units','createdAt']) && typeof item.preview === 'string'
    && item.preview.length <= 64 && !/[\r\n\v\f\u0085\u2028\u2029]/.test(item.preview)
    && !/[\uD800-\uDBFF]$/.test(item.preview) && !/^[\uDC00-\uDFFF]/.test(item.preview);
}
__ASYNC__function errorOf(run, code, method) {
  try { __AWAIT__run(); } catch (error) {
    check(error.code === code && error.method === method && error.effectState === 'not_started', 'expected_error');
    check(!String(error.message).includes('合成正文'), 'error_redaction');
    return;
  }
  check(false, 'missing_error');
}
export __ASYNC__function run(ctx) {
  const h = ctx.clipboard.history;
  check(h && ['list','get','append','remove'].every(name => typeof h[name] === 'function'), 'history_api_dependency');
  const input = ctx.input;
  const op = input.scenario;
  const report = {scenario:op, passed:true, dependencyModules:2};
  if (op === 'baseline') {
    const list = __AWAIT__h.list({limit:200});
    check(exact(list,['items','revision']) && list.revision === '0' && list.items.length === 0, 'fresh_history');
    return {...report, revision:list.revision, count:0};
  }
  if (op === 'append' || op === 'concurrent-append') {
    let result;
    try { result = __AWAIT__h.append({text:sample(input.kind, input.index)}); }
    catch (error) {
      if (op === 'concurrent-append' && error.code === 'history_busy' && error.effectState === 'not_started') return {...report, outcome:'busy'};
      throw error;
    }
    check(exact(result,['id','revision','inserted']), 'append_shape');
    if (op === 'append') check(result.id === input.id && result.revision === input.revision && result.inserted === input.inserted, 'append_counters');
    else check(result.inserted === true, 'concurrent_insert');
    return {...report, outcome:'inserted', ...result};
  }
  if (op === 'get') {
    const item = __AWAIT__h.get({id:input.id});
    check(exact(item,['id','text','createdAt','revision']) && item.id === input.id && item.revision === input.revision, 'get_shape_counters');
    check(item.text === sample(input.kind, input.index), 'get_text_integrity');
    check(/^(0|[1-9][0-9]*)$/.test(item.createdAt), 'timestamp');
    const list = __AWAIT__h.list({limit:200});
    check(list.items.every(validSummary) && !list.items.some(value => Object.prototype.hasOwnProperty.call(value,'text')), 'summary_privacy');
    const summary = list.items.find(value => value.id === item.id);
    check(summary && summary.utf16Units === item.text.length, 'utf16_units');
    if (input.kind === 'unicode') check(summary.preview === 'x'.repeat(63), 'surrogate_boundary');
    return {...report, id:item.id, revision:item.revision, utf16Units:item.text.length, createdAt:item.createdAt};
  }
  if (op === 'remove') {
    const result = __AWAIT__h.remove({id:input.id});
    check(exact(result,['removed','revision']) && result.removed === input.removed && result.revision === input.revision, 'remove_semantics');
    return {...report, ...result};
  }
  if (op === 'missing') {
    __AWAIT__errorOf(() => h.get({id:input.id}), 'history_item_missing', 'history.get');
    return report;
  }
  if (op === 'invalid') {
    const cases = [
      ['history.list', () => h.list({limit:0})], ['history.list', () => h.list({limit:201})],
      ['history.list', () => h.list({limit:1.5})], ['history.list', () => h.list(null)],
      ['history.get', () => h.get({id:'0'})], ['history.get', () => h.get({id:'01'})],
      ['history.remove', () => h.remove({id:'18446744073709551616'})],
      ['history.append', () => h.append({text:'\u0000合成正文'})],
      ['history.append', () => h.append({text:'\uD800'})],
      ['history.append', () => h.append({text:'😀'.repeat(32769)})],
      ['history.append', () => h.append({text:'x',path:'unexpected'})],
    ];
    for (const item of cases) __AWAIT__errorOf(item[1], 'invalid_input', item[0]);
    return {...report, checkedErrors:cases.length};
  }
  if (op === 'batch') {
    for (let i = input.start; i < input.start + input.count; i++) {
      const result = __AWAIT__h.append({text:sample(input.kind, i)});
      check(result.inserted && result.id === String(input.baseId + i) && result.revision === String(input.baseRevision + i), 'batch_counters');
    }
    return {...report, appended:input.count};
  }
  if (op === 'capacity') {
    const list = __AWAIT__h.list({limit:200});
    check(list.revision === input.revision && list.items.length === input.count, 'capacity_count');
    check(list.items.every(validSummary), 'capacity_summary');
    check(list.items[0].id === input.newest && list.items[list.items.length - 1].id === input.oldest, 'capacity_order');
    if (input.totalUnits !== undefined) check(list.items.reduce((sum,item) => sum + item.utf16Units,0) === input.totalUnits, 'capacity_units');
    return {...report, revision:list.revision, count:list.items.length};
  }
  if (op === 'store-error') {
    __AWAIT__errorOf(() => h.list({limit:200}), input.code, 'history.list');
    __AWAIT__errorOf(() => h.get({id:'1'}), input.code, 'history.get');
    __AWAIT__errorOf(() => h.append({text:sample('base')}), input.code, 'history.append');
    __AWAIT__errorOf(() => h.remove({id:'1'}), input.code, 'history.remove');
    return {...report, checkedErrors:4, code:input.code};
  }
  if (op === 'exhausted') {
    __AWAIT__errorOf(() => h.append({text:sample('unicode')}), 'history_id_exhausted', 'history.append');
    __AWAIT__errorOf(() => h.remove({id:'18446744073709551615'}), 'history_revision_exhausted', 'history.remove');
    const duplicate = __AWAIT__h.append({text:sample('base')});
    check(!duplicate.inserted && duplicate.id === '18446744073709551615' && duplicate.revision === '18446744073709551615', 'u64_dedup');
    const missing = __AWAIT__h.remove({id:'1'});
    check(!missing.removed && missing.revision === '18446744073709551615', 'u64_missing_remove');
    return {...report, checkedErrors:2, revision:duplicate.revision};
  }
  if (op === 'revision-exhausted') {
    __AWAIT__errorOf(() => h.append({text:sample('base')}), 'history_revision_exhausted', 'history.append');
    return {...report, checkedErrors:1};
  }
  if (op === 'concurrent-check') {
    const list = __AWAIT__h.list({limit:200});
    check(list.revision === String(input.entries.length) && list.items.length === input.entries.length, 'concurrent_no_lost_updates');
    for (const entry of input.entries) {
      const item = __AWAIT__h.get({id:entry.id});
      check(item.text === sample('concurrent', entry.index), 'concurrent_body_integrity');
    }
    return {...report, revision:list.revision, count:list.items.length};
  }
  check(false, 'unknown_scenario');
}
`;
  checks = checks.replaceAll('__ASYNC__', node ? 'async ' : '').replaceAll('__AWAIT__', node ? 'await ' : '');
  return {'main.mjs':`import {run} from './lib/checks.mjs';\n${node ? 'export default async function main(ctx) { return await run(ctx); }' : 'export function main(ctx) { return run(ctx); }'}\n`,
    'lib/checks.mjs':checks, 'lib/text.mjs':words};
}

function safeEnvelope(child) {
  requireCheck(!child.error && child.signal == null, 'cli_process_failed', {
    timeout:child.error?.code === 'ETIMEDOUT',exitCode:Number.isInteger(child.status) ? child.status : null,
    childPid:Number.isInteger(child.pid) ? child.pid : null,effectState:'unknown',
  });
  requireCheck(!String(child.stdout).includes('合成正文') && !String(child.stderr).includes('合成正文'), 'terminal_body_leak');
  let envelope;
  try { envelope = JSON.parse(child.status === 0 ? child.stdout : child.stderr); }
  catch { throw new CheckFailure('cli_envelope_invalid', {exitCode:child.status}); }
  requireCheck(child.status === 0 && envelope.status === 'succeeded', 'cli_action_failed', diagnostics(envelope.error));
  requireCheck(envelope.output && envelope.output.passed === true && envelope.output.dependencyModules === 2, 'cli_summary_missing');
  const output = envelope.output;
  const permitted = ['scenario','passed','dependencyModules','revision','count','outcome','id','inserted','removed','utf16Units','createdAt','checkedErrors','appended','code'];
  requireCheck(Object.keys(output).every(key => permitted.includes(key)), 'terminal_not_summary');
  return output;
}

function makeFixture(root, name, mode, application) {
  const current = join(root,name);
  const actions = join(current,'actions');
  const folder = join(actions,'history-check');
  const config = join(current,'bindings.json');
  const files = fixtureSources(mode.node);
  return withMetadata([root,current,actions,folder,join(folder,'lib'),config,join(folder,'action.json'),...Object.keys(files).map(path => join(folder,path))], () => {
  const makeDirectory = path => { owned(root,path,{missing:true}); mkdirSync(path); };
  makeDirectory(current); makeDirectory(actions); makeDirectory(folder); makeDirectory(join(folder,'lib'));
  put(root, config, '[]', true);
  for (const [path, source] of Object.entries(files)) put(root, join(folder,path), source, true);
  put(root, join(folder,'action.json'), JSON.stringify({schema_version:1,id:'history-check',name:'合成历史持久化验收',version:'1.0.0',
    actions:[{id:'verify',title:'历史存储验收',version:1,runtime:mode.node?'node':'script',entry:'main.mjs'}]}), true);
  const originals = new Map(Object.keys(files).concat('action.json').map(path => [join(folder,path), digest(read(root,join(folder,path)))]));
  const env = {...process.env, OMNITRIGGER_NODE:process.execPath};
  delete env.OMNITRIGGER_SCRIPT_HOST;
  if (mode.host) env.OMNITRIGGER_SCRIPT_HOST = mode.host;
  let sequence = 0;
  const history = join(current, 'clipboard-history.json');
  function annotate(error, scenario) {
    if (error instanceof CheckFailure) error.detail = {fixture:name,scenario,...error.detail};
    return error;
  }
  function prepare(input) {
    const call = join(current,`call-${sequence+1}.json`);
    return withMetadata([root,application,...(mode.host ? [mode.host] : []),config,...originals.keys(),history,history+'.lock',call], () => {
    requireCheck(sequence < 1000, 'call_bound');
    ordinary(application, {kind:'file'});
    if (mode.host) ordinary(mode.host, {kind:'file'});
    requireCheck(read(root,config).toString() === '[]', 'owned_config_modified');
    for (const [path, hash] of originals) requireCheck(digest(read(root,path)) === hash, 'fixture_dependency_modified');
    owned(root,history,{missing:true,kind:'file'});
    owned(root,history+'.lock',{missing:true,kind:'file'});
    sequence++;
    const bytes = JSON.stringify({package_id:'history-check',action_id:'verify',action_version:1,input});
    requireCheck(Buffer.byteLength(bytes) < 4096, 'call_must_not_embed_long_text');
    put(root,call,bytes,true);
    return {args:['--config',config,'--run-action',call],options:{encoding:'utf8',windowsHide:true,timeout:45000,maxBuffer:128*1024,env}};
    });
  }
  function inspect() {
    return withMetadata([root,current,config,history,history+'.lock',...originals.keys()], () => {
    requireCheck(read(root,config).toString() === '[]', 'owned_config_modified');
    owned(root,current,{kind:'directory'});
    for (const entry of readdirSync(current,{withFileTypes:true})) {
      requireCheck(!entry.isSymbolicLink(), 'unexpected_fixture_link');
      if (entry.name.endsWith('.tmp')) throw new CheckFailure('temporary_content_left_after_terminal');
    }
    for (const [path,hash] of originals) requireCheck(digest(read(root,path)) === hash,'fixture_dependency_modified');
    });
  }
  return {current,history,config,
    invoke(input) {
      try { const {args,options} = prepare(input); const result = safeEnvelope(spawnSync(application,args,options)); inspect(); return result; }
      catch (error) { throw annotate(error,input.scenario); }
    },
    async concurrent(inputs) {
      const prepared = inputs.map(prepare);
      const results = await Promise.allSettled(prepared.map(({args,options},index) => new Promise((resolveResult,rejectResult) => {
        const child = spawn(application,args,options);
        const chunks = {stdout:[],stderr:[]};
        let bytes = 0;
        for (const stream of ['stdout','stderr']) child[stream].on('data',chunk => {
          bytes += chunk.length;
          if (bytes <= options.maxBuffer) chunks[stream].push(chunk);
          else { child.kill(); rejectResult(annotate(new CheckFailure('concurrent_output_bound',{effectState:'unknown',childPid:child.pid}),inputs[index].scenario)); }
        });
        child.on('error',() => rejectResult(new CheckFailure('concurrent_spawn_failed')));
        child.on('close',(status,signal) => {
          try { resolveResult(safeEnvelope({status,signal,pid:child.pid,stdout:Buffer.concat(chunks.stdout).toString(),stderr:Buffer.concat(chunks.stderr).toString()})); }
          catch (error) { rejectResult(annotate(error,inputs[index].scenario)); }
        });
      })));
      const failure = results.find(result => result.status === 'rejected');
      if (failure) throw failure.reason;
      inspect(); return results.map(result => result.value);
    },
  };
  });
}

function preserve(root, fixture, scenario) {
  const before = read(root,fixture.history);
  const result = fixture.invoke(scenario);
  requireCheck(digest(read(root,fixture.history)) === digest(before), 'original_history_bytes_changed');
  return result;
}

async function verifyMode(root, mode, application) {
  const prefix = mode.name;
  const f = makeFixture(root, prefix+'-persistence',mode,application);
  let calls = 0;
  const run = input => { calls++; return f.invoke(input); };
  run({scenario:'baseline'});
  requireCheck(!existsSync(f.history), 'list_created_body_file');
  run({scenario:'append',kind:'base',id:'1',revision:'1',inserted:true});
  const first = run({scenario:'get',kind:'base',id:'1',revision:'1'});
  preserve(root,f,{scenario:'append',kind:'base',id:'1',revision:'1',inserted:false}); calls++;
  const second = run({scenario:'get',kind:'base',id:'1',revision:'1'});
  requireCheck(first.createdAt === second.createdAt, 'dedup_timestamp_changed');
  preserve(root,f,{scenario:'remove',id:'99',removed:false,revision:'1'}); calls++;
  preserve(root,f,{scenario:'invalid'}); calls++;
  run({scenario:'remove',id:'1',removed:true,revision:'2'});
  run({scenario:'missing',id:'1'});
  run({scenario:'append',kind:'empty',id:'2',revision:'3',inserted:true});
  run({scenario:'get',kind:'empty',id:'2',revision:'3'});
  run({scenario:'append',kind:'unicode',id:'3',revision:'4',inserted:true});
  run({scenario:'get',kind:'unicode',id:'3',revision:'4'});
  for (let start = 0; start < 201; start += 25) run({scenario:'batch',kind:'count',start,count:Math.min(25,201-start),baseId:4,baseRevision:5});
  run({scenario:'capacity',revision:'205',count:200,newest:'204',oldest:'5'});
  run({scenario:'missing',id:'4'});
  run({scenario:'get',kind:'count',index:200,id:'204',revision:'205'});

  const large = makeFixture(root,prefix+'-large',mode,application);
  large.invoke({scenario:'baseline'}); calls++;
  for (let index = 0; index < 17; index++) { large.invoke({scenario:'append',kind:'large',index,id:String(index+1),revision:String(index+1),inserted:true}); calls++; }
  large.invoke({scenario:'capacity',revision:'17',count:16,newest:'17',oldest:'2',totalUnits:1048576}); calls++;
  large.invoke({scenario:'missing',id:'1'}); calls++;
  large.invoke({scenario:'get',kind:'large',index:16,id:'17',revision:'17'}); calls++;
  requireCheck(read(root,large.history).length <= 8*1024*1024, 'serialized_storage_limit');

  const exhausted = makeFixture(root,prefix+'-u64',mode,application);
  put(root,exhausted.history,JSON.stringify({version:1,lastId:'18446744073709551614',revision:'18446744073709551614',items:[]}),true);
  exhausted.invoke({scenario:'append',kind:'base',id:'18446744073709551615',revision:'18446744073709551615',inserted:true}); calls++;
  exhausted.invoke({scenario:'get',kind:'base',id:'18446744073709551615',revision:'18446744073709551615'}); calls++;
  preserve(root,exhausted,{scenario:'exhausted'}); calls++;
  const revisionExhausted = makeFixture(root,prefix+'-revision-u64',mode,application);
  put(root,revisionExhausted.history,JSON.stringify({version:1,lastId:'0',revision:'18446744073709551615',items:[]}),true);
  preserve(root,revisionExhausted,{scenario:'revision-exhausted'}); calls++;

  const corruptions = [
    ['invalid-json', '{合成正文'],
    ['unknown-version', JSON.stringify({version:2,lastId:'0',revision:'0',items:[]})],
    ['unknown-field', JSON.stringify({version:1,lastId:'0',revision:'0',items:[],unknown:'合成正文'})],
    ['invalid-unicode', String.raw`{"version":1,"lastId":"1","revision":"1","items":[{"id":"1","text":"\ud800","createdAt":"0"}]}`],
    ['oversized', Buffer.alloc(8*1024*1024+1,0x20)],
  ];
  for (const [name,contents] of corruptions) {
    const current = makeFixture(root,prefix+'-'+name,mode,application);
    put(root,current.history,contents,true);
    preserve(root,current,{scenario:'store-error',code:'history_corrupt'}); calls++;
  }
  const badLock = makeFixture(root,prefix+'-unknown-lock',mode,application);
  put(root,badLock.history,JSON.stringify({version:1,lastId:'0',revision:'0',items:[]}),true);
  put(root,badLock.history+'.lock','unknown synthetic lock',true);
  preserve(root,badLock,{scenario:'store-error',code:'history_corrupt'}); calls++;
  requireCheck(read(root,badLock.history+'.lock').toString() === 'unknown synthetic lock', 'unknown_lock_changed');

  const deleted = makeFixture(root,prefix+'-deleted',mode,application);
  deleted.invoke({scenario:'append',kind:'base',id:'1',revision:'1',inserted:true}); calls++;
  owned(root,deleted.history,{missing:false,kind:'file'}); unlinkSync(deleted.history);
  deleted.invoke({scenario:'store-error',code:'history_conflict'}); calls++;
  requireCheck(!existsSync(deleted.history), 'deleted_history_recreated');

  const parallel = makeFixture(root,prefix+'-concurrency',mode,application);
  parallel.invoke({scenario:'baseline'}); calls++;
  const replies = await parallel.concurrent([0,1,2,3].map(index => ({scenario:'concurrent-append',kind:'concurrent',index}))); calls += 4;
  const entries = replies.flatMap((reply,index) => reply.outcome === 'inserted' ? [{index,id:reply.id}] : []);
  requireCheck(entries.length > 0 && new Set(entries.map(item=>item.id)).size === entries.length, 'concurrent_unique_ids');
  parallel.invoke({scenario:'concurrent-check',entries}); calls++;
  return {mode:mode.name,passed:true,cliCalls:calls,persistence:true,capacityLimits:2,corruptFilesPreserved:corruptions.length,
    successfulConcurrentWrites:entries.length,busyConcurrentWrites:4-entries.length,dependencyModules:2,u64OverflowPreserved:true,
    cancellation:{status:'not_exercised',reason:'cli_has_no_cancellation_endpoint'},noUi:true,noListener:true,noSystemClipboard:true};
}

/** 只解析并检查合成模块，绝不启动应用，也不创建/读取验收存储。 */
export async function selfTest() {
  const metadataCases = [
    [{status:0,stdout:'{"classification":"checked","checkedCount":2}',stderr:''},'checked'],
    [{status:7,stdout:'{"classification":"attributes_rejected","pathIndex":0,"blockedAttributes":4096}',stderr:''},'unsafe_metadata_attributes'],
    [{status:8,stdout:'{"classification":"metadata_error","exceptionType":"UnauthorizedAccessException"}',stderr:''},'metadata_query_failed'],
    [{status:null,error:{code:'ETIMEDOUT'},stderr:'合成正文_不可回显'},'metadata_timeout'],
    [{status:null,error:{code:'ENOENT'},stderr:''},'metadata_spawn_failed'],
    [{status:null,error:{code:'ENOBUFS'},stderr:''},'metadata_output_limit'],
    [{status:1,stdout:'',stderr:'合成正文_不可回显'},'metadata_command_failed'],
    [{status:0,stdout:'not-json',stderr:''},'metadata_protocol_failed'],
  ];
  for (const [input,classification] of metadataCases) {
    const result = classifyMetadataCheck(input,2);
    requireCheck(result.classification === classification && !JSON.stringify(result).includes('合成正文'),'metadata_failure_classification');
  }
  for (const node of [true,false]) {
    const files = fixtureSources(node);
    requireCheck(Object.keys(files).length === 3, 'generated_dependency_count');
    for (const source of Object.values(files)) {
      const parsed = spawnSync(process.execPath,['--check','--input-type=module'], {input:source,encoding:'utf8',windowsHide:true,timeout:10000,maxBuffer:4096});
      requireCheck(!parsed.error && parsed.status === 0, 'generated_source_syntax');
    }
    const sandbox = {};
    vm.createContext(sandbox);
    vm.runInContext(files['lib/text.mjs'].replace('export function','function')+';globalThis.sample = sample;',sandbox);
    requireCheck(sandbox.sample('large',16).length === 65536 && sandbox.sample('large',16).charCodeAt(2) === 1, 'large_text_generation');
    requireCheck(sandbox.sample('empty') === '' && sandbox.sample('base').includes('\r\n'), 'empty_whitespace_generation');
    requireCheck(files['lib/checks.mjs'].includes("from './text.mjs'") && files['main.mjs'].includes("from './lib/checks.mjs'"), 'generated_dependency_chain');
    vm.runInContext(files['lib/checks.mjs'].replace("import {sample} from './text.mjs';",'').replace('export ', '')+';globalThis.run = run;', sandbox);
    const text = sandbox.sample('unicode');
    const history = {list:() => ({items:[{id:'3',preview:'x'.repeat(63),utf16Units:text.length,createdAt:'0'}],revision:'4'}),
      get:() => ({id:'3',text,createdAt:'0',revision:'4'}),append() {},remove() {}};
    const summary = await sandbox.run({input:{scenario:'get',kind:'unicode',id:'3',revision:'4'},clipboard:{history}});
    requireCheck(summary.passed && !JSON.stringify(summary).includes('合成正文'), 'generated_summary_redaction');
    let rejected = false;
    try { await sandbox.run({input:{scenario:'get',kind:'unicode',id:'3',revision:'4'},clipboard:{history:{...history,get:() => ({id:'3',text:'wrong',createdAt:'0',revision:'4'})}}}); }
    catch (error) { rejected = error.message === 'history_check:get_text_integrity'; }
    requireCheck(rejected, 'generated_check_must_detect_corruption');
  }
  return {passed:true,generatedRuntimeModes:2,generatedModulesChecked:6,metadataClassificationCases:metadataCases.length,summaryRedaction:true,negativeAssertion:true,appStarted:false,storageAccessed:false};
}

async function main(args) {
  if (args.length === 1 && args[0] === '--self-test') return selfTest();
  if (args.includes('--help')) return {usage:'node tools/script-sdk-check/check-clipboard-history.mjs [app.exe] [--light-host host.exe]...',
    selfTest:'--self-test 只解析合成模块，不运行应用',artifacts:'仅在项目 .tmp/clipboard-history-cli-* 下创建合成配置/包/历史，保留供检查'};
  requireCheck(process.platform === 'win32','windows_required');
  let application = resolve(repo,'target/debug/omnitrigger.exe');
  const modes = [{name:'node',node:true}];
  if (args.length && !args[0].startsWith('--')) application = resolve(args.shift());
  while (args.length) {
    requireCheck(args.shift() === '--light-host' && args.length > 0 && !args[0].startsWith('--'), 'invalid_arguments');
    modes.push({name:`script-${modes.length}`,node:false,host:resolve(args.shift())});
  }
  ordinary(application,{kind:'file'});
  ordinary(process.execPath,{kind:'file'});
  for (const mode of modes) if (mode.host) ordinary(mode.host,{kind:'file'});
  ordinary(repo,{kind:'directory'});
  const temp = join(repo,'.tmp'); ordinary(temp,{missing:true,kind:'directory'});
  if (!existsSync(temp)) mkdirSync(temp);
  const root = mkdtempSync(join(temp,'clipboard-history-cli-'));
  ordinary(root,{kind:'directory'});
  requireCheck(within(realpathSync(temp),realpathSync(root)), 'owned_root_alias');
  try {
    const checks = [];
    for (const mode of modes) checks.push(await verifyMode(root,mode,application));
    const result = {passed:true,root,checks,personalConfigurationAccessed:false,appUiStarted:false,observerStarted:false};
    put(root,join(root,'result.json'),JSON.stringify(result,null,2),true);
    return result;
  } catch (error) {
    const result = {passed:false,root,stage:error instanceof CheckFailure ? error.stage : 'unexpected_check_failure',
      diagnostic:error instanceof CheckFailure ? error.detail : {},rawActionOutputSaved:false};
    put(root,join(root,'result.json'),JSON.stringify(result,null,2),true);
    throw new CheckFailure(result.stage,result);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(result => console.log(JSON.stringify(result))).catch(error => {
    console.error(JSON.stringify({passed:false,stage:error instanceof CheckFailure ? error.stage : 'unexpected_check_failure',
      diagnostic:error instanceof CheckFailure ? error.detail : {}}));
    process.exitCode = 1;
  });
}
