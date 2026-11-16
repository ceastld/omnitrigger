// 普通入口的正负类型调用合同，不属于运行时模块闭包。
import type {SessionInput,SessionEvent,SessionResult} from './session-types';
import {main} from './session-step-script.mjs';
const create:SessionInput={mode:'create',initial:{sessionId:'typed',text:'{"n":9007199254740993}',continuous:true}};
const created:SessionResult=main({input:create});
const select:SessionInput={mode:'step',state:created.state,event:{type:'select',revision:created.state.revision,path:['n']}};
const selected:SessionResult=main({input:select});
const ack:SessionEvent={type:'ack_output',requestId:'typed:1',status:'unknown'};
// @ts-expect-error 文档必须为JSON文本，不接受可能已经舍入的JS对象。
const badText:SessionInput={mode:'create',initial:{sessionId:'typed',text:{n:1},continuous:true}};
// @ts-expect-error 整文档输出始终是JSON，不能用raw-string去引号。
const badFormat:SessionEvent={type:'request_output',revision:0,kind:'document',format:'raw-string'};
// @ts-expect-error revision不是BigInt或字符串。
const badRevision:SessionEvent={type:'home',revision:1n};
// @ts-expect-error 原JSON-Path未实现，不能宣告为Pointer以外路径格式。
const badPathFormat:SessionEvent={type:'request_output',revision:0,kind:'path',format:'jsonpath'};
