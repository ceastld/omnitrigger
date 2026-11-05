// 只读排障：必须显式传入历史路径；不推断配置、不打印正文、不获取/创建写锁。
import {spawnSync} from 'node:child_process';
import {closeSync, constants, fstatSync, lstatSync, openSync, opendirSync, readSync, statfsSync} from 'node:fs';
import {basename, dirname, isAbsolute, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const MAX_BYTES = 8 * 1024 * 1024;
const MAX_ITEMS = 200;
const MAX_UNITS = 1048576;
const MAX_TEMPORARIES = 8;
const MAX_DIRECTORY_ENTRIES = 1000;
const LOCK_MAGIC = 'OmniTrigger.ClipboardHistory.Lock.v1\n';
const MAX_U64 = '18446744073709551615';
const deniedAttributes = 0x441400;

class InspectionFault extends Error {
  constructor(code) { super(code); this.code = code; }
}
function requireState(ok, code) { if (!ok) throw new InspectionFault(code); }
export function unsafeAttributes(attributes) { return (attributes & deniedAttributes) !== 0; }
function ioCode(error) {
  if (error instanceof InspectionFault) return error.code;
  if (error?.code === 'ENOENT') return 'file_missing';
  if (['EACCES','EPERM','EBUSY'].includes(error?.code)) return 'access_unavailable';
  return 'io_unavailable';
}

function absoluteFile(path) {
  requireState(typeof path === 'string' && path.length > 0 && !path.includes('\0') && isAbsolute(path), 'explicit_absolute_file_required');
  requireState(!path.split(/[\\/]/).some(part => part === '.' || part === '..'), 'noncanonical_path');
  if (process.platform === 'win32') {
    requireState(/^[a-z]:[\\/]/i.test(path) && !path.slice(2).includes(':'), 'ordinary_local_disk_required');
  }
  const normalized = resolve(path);
  requireState(dirname(normalized) !== normalized, 'file_name_required');
  return normalized;
}

/** 只读取明确祖先元数据；Windows 先检查 Offline/Recall/Reparse，再打开正文。 */
export function checkPath(path, {allowMissingFinal = false, directory = false} = {}) {
  const target = resolve(path);
  const paths = [target];
  while (dirname(paths[0]) !== paths[0]) paths.unshift(dirname(paths[0]));
  requireState(paths.every(value => {
    const name = basename(value).toLowerCase();
    return !name.startsWith('onedrive') && name !== 'dropbox' && !name.startsWith('icloud');
  }), 'cloud_path_rejected');
  if (process.platform === 'win32') {
    const source = `$ErrorActionPreference='Stop'; try { $paths=ConvertFrom-Json $env:OMNITRIGGER_HISTORY_INSPECT_PATHS; foreach($p in $paths) { if(Test-Path -LiteralPath $p) { $item=Get-Item -LiteralPath $p -Force; if(([long]$item.Attributes -band 0x441400) -ne 0) { exit 7 } } }; exit 0 } catch { exit 8 }`;
    const result = spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(source,'utf16le').toString('base64')], {
      windowsHide:true,encoding:'utf8',timeout:15000,maxBuffer:4096,
      env:{...process.env,OMNITRIGGER_HISTORY_INSPECT_PATHS:JSON.stringify(paths)},
    });
    requireState(!result.error && result.status === 0, result.status === 7 ? 'unavailable_attributes_rejected' : 'metadata_unavailable');
  }
  let final;
  for (const current of paths) {
    let metadata;
    try { metadata = lstatSync(current,{bigint:true}); }
    catch (error) {
      if (error.code === 'ENOENT' && current === target && allowMissingFinal) return null;
      throw new InspectionFault(current === target ? ioCode(error) : 'parent_unavailable');
    }
    requireState(!metadata.isSymbolicLink(), 'link_rejected');
    if (current === target) {
      requireState(!metadata.isFile() || metadata.nlink <= 1n,'hard_link_rejected');
      final = metadata;
    }
    else requireState(metadata.isDirectory(), 'parent_not_directory');
  }
  requireState(directory ? final.isDirectory() : final.isFile(), directory ? 'not_directory' : 'not_regular_file');
  return final;
}

function identity(metadata) {
  return [metadata.dev,metadata.ino,metadata.size,metadata.mtimeNs,metadata.ctimeNs].map(String).join(':');
}

function readBounded(path, maximum) {
  const before = checkPath(path,{allowMissingFinal:true});
  if (!before) return {state:'missing'};
  if (before.size > BigInt(maximum)) return {state:'oversized',bytes:before.size.toString()};
  let descriptor;
  try {
    descriptor = openSync(path,constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    const opened = fstatSync(descriptor,{bigint:true});
    requireState(opened.isFile() && identity(before) === identity(opened), 'changed_during_read');
    const buffer = Buffer.alloc(Math.min(maximum+1,Number(before.size)+1));
    let length = 0;
    while (length < buffer.length) {
      const count = readSync(descriptor,buffer,length,buffer.length-length,null);
      if (count === 0) break;
      length += count;
    }
    const afterHandle = fstatSync(descriptor,{bigint:true});
    const afterPath = checkPath(path);
    requireState(identity(opened) === identity(afterHandle) && identity(opened) === identity(afterPath), 'changed_during_read');
    if (length > maximum) return {state:'oversized',bytes:String(length)};
    return {state:'read',bytes:length,buffer:buffer.subarray(0,length)};
  } finally { if (descriptor !== undefined) closeSync(descriptor); }
}

function decimal(value, zero = true) {
  return typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value)
    && (zero || value !== '0') && (value.length < 20 || value <= MAX_U64);
}
function keys(value, names) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value,name));
}
function text(value) {
  if (typeof value !== 'string' || value.length > 65536 || value.includes('\0')) return false;
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++index);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}

// JSON.parse 会吞掉重复字段；另做有界结构扫描，不保留或回显字段/正文片段。
function strictStructure(source) {
  let at = 0;
  let tokens = 0;
  let versionLexeme;
  const white = () => { while (at < source.length && /[ \r\n\t]/.test(source[at])) at++; };
  function string() {
    const start = at++;
    while (at < source.length) {
      const character = source[at++];
      if (character === '\\') at++;
      else if (character === '"') return source.slice(start,at);
    }
    throw new InspectionFault('invalid_json');
  }
  function value(depth) {
    requireState(depth <= 16 && ++tokens <= 4096,'json_complexity_limit');
    white();
    if (source[at] === '"') { string(); return; }
    if (source[at] === '{') {
      at++; white();
      const seen = new Set();
      if (source[at] === '}') { at++; return; }
      for (;;) {
        white();
        const key = JSON.parse(string());
        requireState(!seen.has(key),'duplicate_field');
        seen.add(key); white(); at++; white();
        const start = at;
        value(depth+1);
        if (depth === 0 && key === 'version') versionLexeme = source.slice(start,at).trim();
        white();
        if (source[at++] === '}') return;
      }
    }
    if (source[at] === '[') {
      at++; white();
      if (source[at] === ']') { at++; return; }
      for (;;) { value(depth+1); white(); if (source[at++] === ']') return; }
    }
    while (at < source.length && !/[\],} \r\n\t]/.test(source[at])) at++;
  }
  value(0);
  return versionLexeme;
}

/** 返回只有计数和规范标识的摘要；不返回 text、preview、字段内容或正文指纹。 */
export function inspectDocument(bytes) {
  try {
    requireState(bytes.length <= MAX_BYTES,'storage_limit');
    let source;
    try { source = new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes); }
    catch { throw new InspectionFault('invalid_utf8'); }
    let document;
    try { document = JSON.parse(source); }
    catch { throw new InspectionFault('invalid_json'); }
    const versionLexeme = strictStructure(source);
    requireState(keys(document,['version','lastId','revision','items']),'document_fields');
    requireState(document.version === 1 && versionLexeme === '1','unsupported_version');
    requireState(decimal(document.lastId) && decimal(document.revision),'invalid_counter');
    requireState(BigInt(document.revision) >= BigInt(document.lastId),'revision_before_last_id');
    requireState(Array.isArray(document.items) && document.items.length <= MAX_ITEMS,'item_count_limit');
    let previous = 0n;
    let units = 0;
    for (const item of document.items) {
      requireState(keys(item,['id','text','createdAt']),'item_fields');
      requireState(decimal(item.id,false) && decimal(item.createdAt),'invalid_item_metadata');
      const id = BigInt(item.id);
      requireState(id > previous && id <= BigInt(document.lastId),'invalid_item_order');
      previous = id;
      requireState(text(item.text),'invalid_text');
      units += item.text.length;
      requireState(units <= MAX_UNITS,'total_units_limit');
    }
    return {state:'valid',schemaVersion:1,lastId:document.lastId,revision:document.revision,itemCount:document.items.length,totalUtf16Units:units,
      oldestId:document.items[0]?.id ?? null,newestId:document.items.at(-1)?.id ?? null};
  } catch (error) { return {state:'corrupt',reason:error instanceof InspectionFault ? error.code : 'invalid_document'}; }
}

function historySnapshot(path) {
  try {
    const snapshot = readBounded(path,MAX_BYTES);
    if (snapshot.state !== 'read') return snapshot;
    return {...inspectDocument(snapshot.buffer),bytes:snapshot.bytes};
  } catch (error) { return {state:'unavailable',reason:ioCode(error)}; }
}
function lockSnapshot(path) {
  try {
    const snapshot = readBounded(path,LOCK_MAGIC.length+2);
    if (snapshot.state !== 'read') return snapshot;
    if (snapshot.buffer.equals(Buffer.from(LOCK_MAGIC+'0\n'))) return {state:'fresh'};
    if (snapshot.buffer.equals(Buffer.from(LOCK_MAGIC+'1\n'))) return {state:'initialized'};
    return {state:'corrupt',reason:'unrecognized_lock_marker'};
  } catch (error) { return {state:'unavailable',reason:ioCode(error)}; }
}

function temporaries(path) {
  const result = {requested:true,scannedEntries:0,complete:true,candidates:[]};
  let directory;
  try {
    checkPath(dirname(path),{directory:true});
    directory = opendirSync(dirname(path));
    const prefix = basename(path)+'.';
    for (;;) {
      if (result.scannedEntries >= MAX_DIRECTORY_ENTRIES) { result.complete = false; break; }
      const entry = directory.readSync();
      if (!entry) break;
      result.scannedEntries++;
      if (!entry.name.startsWith(prefix)) continue;
      const suffix = entry.name.slice(prefix.length);
      const match = /^([1-9][0-9]{0,9})\.([1-9][0-9]{0,19})\.tmp$/.exec(suffix);
      if (!match || Number(match[1]) > 4294967295 || !decimal(match[2],false)) continue;
      if (result.candidates.length === MAX_TEMPORARIES) { result.complete = false; break; }
      // PID 只是文件名元数据，不能证明创建者仍存活、已退出或适合恢复。
      result.candidates.push({suffix,processId:match[1],nonce:match[2],...historySnapshot(join(dirname(path),entry.name))});
    }
    return result;
  } catch (error) { return {...result,complete:false,reason:ioCode(error)}; }
  finally { directory?.closeSync(); }
}

function diskSnapshot(parent) {
  try {
    checkPath(parent,{directory:true});
    const fs = statfsSync(parent,{bigint:true});
    const bytes = fs.bavail*fs.bsize;
    return {state:'observed',availableBytes:bytes.toString(),lessThanMaximumTemporary:bytes < BigInt(MAX_BYTES),provesPreviousWriteFailure:false};
  } catch (error) { return {state:'unavailable',reason:ioCode(error)}; }
}

/** 不加写锁、不写文件；仅为并发未锁定观察，手动恢复前必须另行停止写入者。 */
export function inspectHistory(explicitPath, {scanTemporaries = false} = {}) {
  let path;
  try { path = absoluteFile(explicitPath); checkPath(dirname(path),{directory:true}); }
  catch (error) { return {schemaVersion:1,readOnly:true,state:'unavailable',reason:ioCode(error),bodyRedacted:true}; }
  const history = historySnapshot(path);
  const lock = lockSnapshot(path+'.lock');
  const temporary = scanTemporaries ? temporaries(path) : {requested:false};
  const findings = [];
  if (history.state === 'missing' && lock.state === 'initialized') findings.push('missing_initialized_history_preserve_sidecar');
  if (['corrupt','oversized'].includes(history.state)) findings.push('preserve_corrupt_history_and_evidence');
  if (['corrupt','oversized'].includes(lock.state)) findings.push('preserve_unrecognized_lock');
  if (history.state === 'valid' && lock.state === 'missing') findings.push('valid_history_has_no_sidecar_observed');
  if (temporary.candidates?.length) findings.push('temporary_candidates_require_stopped_writers_and_manual_review');
  if (temporary.requested && !temporary.complete) findings.push('temporary_scan_incomplete');
  if ([history,lock].some(value=>value.state === 'unavailable')) findings.push('observation_incomplete');
  const disk = diskSnapshot(dirname(path));
  if (disk.lessThanMaximumTemporary) findings.push('low_space_observed_without_failure_proof');
  return {schemaVersion:1,readOnly:true,state:findings.length ? 'attention' : 'observed',bodyRedacted:true,
    consistency:'unlocked_observation',writerLiveness:'unknown',history,lock,temporaries:temporary,disk,findings,
    recovery:{automatic:false,mayOverwrite:false,mayDelete:false,mayResetIds:false}};
}

function main(args) {
  if (args.length === 1 && args[0] === '--help') return {usage:'node tools/script-sdk-check/inspect-clipboard-history.mjs --file <绝对历史文件路径> [--scan-temporaries]',
    behavior:'仅只读检查显式文件、sidecar及可选同名临时文件；不打印正文，不自动清理、覆盖或重置ID。'};
  let file;
  let scanTemporaries = false;
  while (args.length) {
    const option = args.shift();
    if (option === '--file' && !file && args.length && !args[0].startsWith('--')) file = args.shift();
    else if (option === '--scan-temporaries' && !scanTemporaries) scanTemporaries = true;
    else throw new InspectionFault('invalid_arguments');
  }
  requireState(file !== undefined,'explicit_absolute_file_required');
  return inspectHistory(file,{scanTemporaries});
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const report = main(process.argv.slice(2));
    console.log(JSON.stringify(report));
    if (['unavailable','attention'].includes(report.state)) process.exitCode = 2;
  } catch (error) {
    console.error(JSON.stringify({readOnly:true,state:'unavailable',reason:ioCode(error),bodyRedacted:true}));
    process.exitCode = 2;
  }
}
