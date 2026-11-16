import type {ActionContext} from '../sdk/api';
import type {DuplicateInput,DuplicateReceipt} from '../workflows.mjs';
import nodeEntry from '../duplicate-tab.mjs';
import {main} from '../duplicate-tab-script.mjs';
import {runDuplicateTabSync} from '../workflows.mjs';
declare const syncContext:ScriptContext<DuplicateInput>;
declare const asyncContext:ActionContext;
const entry:ScriptMain<DuplicateInput>=main;
const json:ScriptJsonValue=main(syncContext);
const receipt:DuplicateReceipt=runDuplicateTabSync(syncContext);
const promise:Promise<DuplicateReceipt>=nodeEntry(asyncContext);
const input:DuplicateInput={profile:'own',windowId:0,tabId:2147483647,focusWindow:true,timeoutMs:1000};
void entry;void json;void receipt;void promise;void input;
// @ts-expect-error 同步入口不能返回Promise。
const forbidden:Promise<unknown>=main(syncContext);
void forbidden;
// @ts-expect-error Node入口不能直接当同步JSON。
const forbiddenJson:ScriptJsonValue=nodeEntry(asyncContext);
void forbiddenJson;
// @ts-expect-error Node异步browser不能代替Script同步browser。
main({...syncContext,browser:asyncContext.browser});
// @ts-expect-error Script同步browser不能代替Node异步browser。
nodeEntry({...asyncContext,browser:syncContext.browser});
// @ts-expect-error 标签ID必须为number。
const badId:DuplicateInput={windowId:4,tabId:'7'};
void badId;
// @ts-expect-error 复制不增加activate参数。
const badActivate:DuplicateInput={windowId:4,tabId:7,activate:true};
void badActivate;
// @ts-expect-error 聚焦只接受boolean。
const badFocus:DuplicateInput={windowId:4,tabId:7,focusWindow:'true'};
void badFocus;
