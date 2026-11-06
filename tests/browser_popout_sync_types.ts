import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
import {main} from '../examples/actions/browser-popout/pop-out-tab-script.mjs';

declare const syncContext:ScriptContext;
declare const asyncContext:ActionContext;
const entry:ScriptMain=main;
const value:ScriptJsonValue=main(syncContext);
void entry;void value;
// @ts-expect-error 同步入口直接返回JSON，不能作为Promise。
const promise:Promise<unknown>=main(syncContext);
void promise;
// @ts-expect-error Node Promise浏览器API不能充当Script同步browser。
const browser:ScriptContext['browser']=asyncContext.browser;
void browser;
// @ts-expect-error 完整ScriptContext也不能以异步browser替换。
main({...syncContext,browser:asyncContext.browser});
