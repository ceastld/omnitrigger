import type {ActionContext, SelectionTextSnapshot} from '../crates/omnitrigger-actions/runtime/api';
declare global {interface AbortSignal {readonly aborted:boolean; readonly reason:unknown;}}
declare const node:ActionContext;
declare const script:ScriptContext;
async function readNode(){
  const pending:Promise<SelectionTextSnapshot>=node.desktop.readSelectionText();
  const result=await pending;
  if(result.text===''){const query:string=result.text;const flag:boolean=result.truncated;void query;void flag;}
  await node.desktop.readSelectionText({overflow:'truncate'});
  // @ts-expect-error Node需要await，不能把Promise当快照。
  node.desktop.readSelectionText().text;
  // @ts-expect-error 成功快照只读。
  result.text='';
}
const result:ScriptSelectionTextSnapshot=script.desktop.readSelectionText({overflow:'reject'});
const syncText:string=script.desktop.readSelectionText({overflow:'truncate'}).text;
if(result.text===''){const query:string=result.text;void query;}
// @ts-expect-error 同步结果不是Promise。
const promise:Promise<ScriptSelectionTextSnapshot>=script.desktop.readSelectionText();
// @ts-expect-error 空值仍为string，不是null。
const empty:null=result.text;
// @ts-expect-error 成功快照元数据只读。
result.truncated=false;
// @ts-expect-error 不允许有损或自动策略。
node.desktop.readSelectionText({overflow:'lossy'});
// @ts-expect-error 没有可变长度选项。
script.desktop.readSelectionText({maxLength:128});
// @ts-expect-error 失败通过拒绝/抛错表示，不伪装成成功值的error字段。
result.error;
void readNode;void syncText;void promise;void empty;
