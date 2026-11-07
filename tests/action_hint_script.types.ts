declare const ctx:ScriptContext;
const result=ctx.ui.setActionHint({text:'v123'});
const updated:true=result.updated;
const snapshot=ctx.state.read('session','hint');
ctx.state.compareAndSwap('durable','hint',snapshot.revision,{present:true,value:null});
ctx.actions.stopOtherInstances({timeoutMs:1});
// @ts-expect-error Script同步回执不是Promise。
result.then;
// @ts-expect-error 同步回执不能作为Promise。
const promise:Promise<unknown>=ctx.ui.setActionHint({text:'v123'});
// @ts-expect-error 不接受自报scope。
ctx.ui.setActionHint({text:'v123',scope:{}});
// @ts-expect-error 不能指定包身份。
ctx.ui.setActionHint({text:'v123',packageId:'other'});
// @ts-expect-error text必须为字符串。
ctx.ui.setActionHint({text:false});
void updated;void promise;
