import type {ActionContext,LocalDirectoryResult,LocalDirectoryEntry,DirectoryReadFault} from '../crates/omnitrigger-actions/runtime/api';
declare const ctx:ActionContext;
declare const script:ScriptContext;
const promise:Promise<LocalDirectoryResult> = ctx.fs.readDirectory('D:/synthetic',{maxEntries:128});
const sync:ScriptLocalDirectoryResult = script.fs.readDirectory('D:/synthetic',{maxEntries:1});
const entries:ReadonlyArray<LocalDirectoryEntry> = sync.entries;
declare const error:DirectoryReadFault;
const method:'fs.read_directory'=error.method;
const effect:'not_started'=error.effectState;
// @ts-expect-error 同步回执不是 Promise。
const wrong:Promise<LocalDirectoryResult> = script.fs.readDirectory('D:/synthetic');
// @ts-expect-error Node 回执必须 await。
const wrongSync:LocalDirectoryResult=ctx.fs.readDirectory('D:/synthetic');
// @ts-expect-error 上限不是字符串。
ctx.fs.readDirectory('D:/synthetic',{maxEntries:'1'});
// @ts-expect-error 递归不属于此 API。
script.fs.readDirectory('D:/synthetic',{recursive:true});
// @ts-expect-error 回执数组不可修改。
sync.entries.push(entries[0]);
// @ts-expect-error 子项不可修改。
sync.entries[0].path='D:/elsewhere';
void [promise,sync,method,effect];
