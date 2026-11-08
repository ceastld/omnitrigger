# 停止当前动作的其他调用

`ctx.actions.stopOtherInstances({timeoutMs:500})` 由 `ActionService` 为当前真实调用提供。Node 返回 Promise，必须 `await`；Script 同步等待。参数可省略，默认 500 毫秒，整数范围为 1–2000 毫秒。参数只接受 `timeoutMs`，不能指定包、动作、绑定或调用身份。

服务在同一锁内找到同一服务、同一包 ID、同一动作 ID 的其他活跃调用，排除自身。它先向整个集合发出取消请求，再等待这些调用的运行时和本次宿主资源清理结束；取消请求、脚本返回和界面读取终态都不能代替资源清理。一次最多涉及三个其他调用。

```js
// Node 动作入口
export default async function main(ctx) {
  const receipt = await ctx.actions.stopOtherInstances({timeoutMs:1000});
  return {stoppedCount:receipt.stoppedCount};
}
```

Script 使用相同方法和参数，同步取得回执：

```js
export function main(ctx) {
  return ctx.actions.stopOtherInstances({timeoutMs:1000});
}
```

没有其他调用时返回 `{stoppedCount:0,effectState:'not_started'}`。向其他调用发出过取消请求并确认清理后，返回 `{stoppedCount:1|2|3,effectState:'unknown'}`。`unknown` 表示这些调用先前可能已产生副作用，停止不能撤销它们。回执仅涵盖这次选中的集合，不承诺未来没有新调用进入，也不提供跨进程单例或事务。

需要从同绑定再次进入业务判断时，动作清单须显式设置 `allow_binding_reentry: true`；普通提交会读取此策略，接纳不自动取消旧调用。该字段不能与 `single_instance: true` 同时启用；默认 false、单实例、配额及未读终态规则见[动作包说明](ACTION_PACKAGES.md)。`stopOtherInstances` 的同包同动作范围仍可跨绑定和动作版本，不随再入策略收窄。

现有 `single_instance` 接纳规则保持不变。当前调用声明该策略时，此 API 返回 `action_policy_conflict`；遇到同动作的活跃单例调用返回 `action_busy`，两者均为 `not_started`。它不会暗中替换单例调用，也不会为了停止而释放绑定或执行配额。

等待受参数预算和外层动作期限中较早者约束。等待预算耗尽为 `stop_timeout`，外层期限耗尽为 `timeout`，取消或桥接断连为 `cancelled`。已发出取消请求后发生这些错误，效果状态为 `unknown`；调用方应终止后续依赖清理成功的操作，不自动重试或补发输入。失效作用域返回 `action_inactive`。

直接使用运行时或独立脚本 CLI，没有服务创建的调用作用域时，方法仍存在，但明确返回 `actions_unavailable` / `not_started`。可读的 `ctx.invocation` 元数据不构成作用域。服务侧由原生桥接绑定实际调用，脚本不通过请求参数选择目标。该本地宿主协议不是针对恶意脚本的安全沙盒。

轻量宿主通过 `actions.stop_other_instances.v1` 声明接口契约。能力描述只表示该版本实现了接口；实际执行仍要求当前服务调用作用域和可用桥接。要求此能力的集成应显式声明要求，旧宿主缺失能力时在执行前报错。
