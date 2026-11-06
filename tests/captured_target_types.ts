import type {ActionContext, CapturedTarget} from '../crates/omnitrigger-actions/runtime/api';
declare global {interface AbortSignal {readonly aborted:boolean; readonly reason:unknown;}}
declare const node: ActionContext;
declare const script: ScriptContext;
const options = {expectedProcessId:20, expectedProcessCreatedFileTime:'134000000000000002', validForMs:500};
async function nodeExample() {
  const target = await node.desktop.captureForegroundTarget(options);
  await node.desktop.sendShortcutTo(target, [163,76], {keyDownDelayMs:0});
  await node.desktop.sendTextTo(target, '合成文本');
  // @ts-expect-error 捕获上下文只读。
  target.context.process_id = 99;
  // @ts-expect-error Node 捕获必须先等待，不能把 Promise 作为目标。
  node.desktop.sendTextTo(node.desktop.captureForegroundTarget(options), 'x');
}
const target = script.desktop.captureForegroundTarget(options);
script.desktop.sendTextTo(target, '合成文本');
script.desktop.sendShortcutTo(target, [163,76]);
// @ts-expect-error 原目标上下文不是新捕获的租约。
script.desktop.sendTextTo(script.desktop.targetContext(), 'x');
// @ts-expect-error 进程创建时间必须保留十进制字符串精度。
script.desktop.captureForegroundTarget({...options, expectedProcessCreatedFileTime:134000000000000002});
// @ts-expect-error 必须明确预期 PID。
script.desktop.captureForegroundTarget({expectedProcessCreatedFileTime:'1', validForMs:500});
// @ts-expect-error 必须明确有效期。
script.desktop.captureForegroundTarget({expectedProcessId:20, expectedProcessCreatedFileTime:'1'});
// @ts-expect-error 此窄接口不接受另一个时序或进程策略。
script.desktop.sendShortcutTo(target, [163,76], {timing:{}});
// @ts-expect-error 普通对象无法充当 API 返回的只读品牌类型。
const forged: CapturedTarget = {capture_id:1, context:{window_handle:'1', process_id:20, thread_id:21, process_created_file_time:'1', expires_at_unix_ms:1000}};
void nodeExample; void forged;
