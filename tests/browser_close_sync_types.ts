import type {ActionContext} from '../examples/actions/browser-close/sdk/api';
import closeNode from '../examples/actions/browser-close/close-tab.mjs';
import {main} from '../examples/actions/browser-close/close-tab-script.mjs';
declare const asyncContext:ActionContext;
declare const syncContext:ScriptContext;
const syncResult:ScriptJsonValue=main(syncContext);
const asyncResult:Promise<unknown>=closeNode(asyncContext);
void syncResult;void asyncResult;
// @ts-expect-error Promise版profiles/call不能作为同步API传入。
main(asyncContext);
// @ts-expect-error 同步入口返回JSON，不能当作Promise。
const forbiddenPromise:Promise<unknown>=main(syncContext);
void forbiddenPromise;
