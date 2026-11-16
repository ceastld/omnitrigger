# 动作版本提示

带界面的动作调用可使用 `ctx.ui.setActionHint({text:'beta'})` 更新自身的版本提示。Node 需要 `await`；Script 同步返回。文本为 1–16 个 UTF-16 单位，不能留首尾空白、控制字符或额外的样式、身份参数。

提示显示在现有动作卡片和结果区域，透明背景、绿色文字。`{updated:true}` 只确认本次调用的界面模型已更新并请求刷新，不代表动作业务成功，也不确认像素已经呈现。

宿主从当前动作服务调用注入绑定、调用和动作版本身份。脚本不能更新其他调用；调用结束后不能再写。提示复用现有内存记录，随最近四条结果淘汰，不写入磁盘。

没有所属界面时返回结构化失败，不能用独立 CLI 检查证明提示已显示。请求通道繁忙、调用身份过期、取消或超时均返回错误；已经更新但回执未确认时效果为 `unknown`，不要自动重试业务。

## 参数

只接受一个 `{text:string}` 对象。文本必须是 1–16 个 UTF-16 单位的单行非空字符串，没有首尾空白；拒绝控制字符、换行分隔符、BOM 和无效代理项。不截断、不 trim、不做 Unicode 归一化。例如 8 个 `🙂` 占 16 个 UTF-16 单位，9 个超过限制；长度不是字形数量。

例如 `v123`、`n10`、`pre` 都可作为显示文字。它们不修改清单中的动作版本，也不改变本次调用的真实身份。空字符串不能用于清除提示。不接受 scope、包/动作/绑定/调用 ID、颜色、位置、时长或进度参数。类型声明检查字段与同步/异步语义；长度及 Unicode 等边界由运行时校验。

```js
/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  const receipt = await ctx.ui.setActionHint({text:'v123'});
  return {hintUpdated:receipt.updated};
}
```

同步 Script 使用相同参数：

```js
/** @type {ScriptMain} */
export function main(ctx) {
  return ctx.ui.setActionHint({text:'v123'});
}
```

## 作用域与界面

服务从当前活跃调用保留项取得真实绑定、调用、包、动作和动作版本；界面再次核对仍活跃的对应反馈项。脚本不能通过可读的 `ctx.invocation`、输入或自报 scope 选择别人的提示。内部转交方法 `ui.action_hint.scoped` 不属于脚本公开协议。

独立运行时、普通 CLI 或没有真实服务调用作用域时，明确报 `ui_hint_scope_expired`。有服务作用域但没有当前调用所属的可用界面、窗口已关闭或界面通道不可用时，报 `ui_unavailable`。接口存在、SDK 类型通过和能力自述均不表示 UI 已就绪。

提示调用可以在没有桌面目标上下文的服务调用中使用，但仍要求真实作用域和 UI。它不启用 desktop，不抓取前台窗口，也不授予发送按键、读取剪贴板、浏览器或其他平台能力。

提示保存在当前界面的有限反馈模型中，不写入持久业务状态。调用结束后不能再更新；最后的提示可随有限完成反馈保留，不承诺结束时立即消失。需要 session/durable 业务状态时，使用[动作状态读写](ACTION_STATE.md)；需要停止其他调用时，使用[动作调用作用域](ACTIONS_SCOPE.md)。这些操作与提示更新没有事务关系。

## 失败和确认边界

错误保留 `code`、`method:'ui.action_hint'` 和 `effectState`。参数无效为 `invalid_input`；作用域结束、身份或版本不匹配为 `ui_hint_scope_expired`；界面通道忙为 `ui_busy`，无可用 UI 为 `ui_unavailable`。原生通道的并发及调用次数限制仍生效，不能从错误名称推导桌面权限。

应用 UI 请求预算最多 1000 毫秒，并受外层较早期限约束。明确尚未应用模型的取消、超时或拒绝为 `not_started`；模型可能已应用后才观察到取消、超时，或回执无效时，保守为 `unknown`。不能据此声称已回滚，SDK不自动重试。提示更新也不代表其前后业务操作成功；动作失败时不要据此伪报完成。

轻量宿主用 `ui.action_hint.v1` 声明接口合同。当前 Windows 自述共 8 项能力，非 Windows 共 5 项且不声明此提示能力；已有 `state.read_cas.v1` 与 `actions.stop_other_instances.v1` 保留。能力数量和接口可用性不是 UI 渲染验收，应分别核对实际宿主与界面状态。
