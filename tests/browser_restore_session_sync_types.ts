import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
import nodeEntry from '../examples/actions/browser-workflows/restore-session.mjs';
import {main} from '../examples/actions/browser-workflows/restore-session-script.mjs';
declare const syncContext:ScriptContext;
declare const asyncContext:ActionContext;
const entry:ScriptMain=main;
const json:ScriptJsonValue=main(syncContext);
const promise:Promise<unknown>=nodeEntry(asyncContext);
void entry;void json;void promise;
// @ts-expect-error 同步入口不得返回Promise。
const forbidden:Promise<unknown>=main(syncContext);
void forbidden;
// @ts-expect-error Node异步browser不能替换同步ScriptContext.browser。
main({...syncContext,browser:asyncContext.browser});
