import type {ActionContext,SelectionTextSnapshot} from '../crates/omnitrigger-actions/runtime/api';
declare const node:ActionContext;
declare const script:ScriptContext;
const asyncValue:Promise<SelectionTextSnapshot>=node.desktop.readSelectionText({maxUtf16Units:8192,overflow:'reject'});
const syncValue:ScriptSelectionTextSnapshot=script.desktop.readSelectionText({maxUtf16Units:2048,overflow:'truncate'});
// @ts-expect-error 上限必须为数值，运行时另外校验整数范围。
node.desktop.readSelectionText({maxUtf16Units:'8192'});
// @ts-expect-error 不接受旧的虚构字段。
script.desktop.readSelectionText({maxLength:8192});
// @ts-expect-error Script仍同步。
const wrongPromise:Promise<SelectionTextSnapshot>=script.desktop.readSelectionText({maxUtf16Units:8192});
// @ts-expect-error Node仍异步。
const wrongSync:SelectionTextSnapshot=node.desktop.readSelectionText({maxUtf16Units:8192});
void [asyncValue,syncValue];
