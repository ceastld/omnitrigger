# 当前动作的状态读写

`ctx.state` 提供当前真实动作的状态快照与比较交换（CAS）。Node 方法返回 Promise，必须 `await`；Script 同步返回结果。两者使用同一服务存储，不在各自的 JavaScript 模块里保存副本。接口要求服务显式提供状态存储；方法存在或宿主声明 `state.read_cas.v1`，不代表本次调用有可用存储。

## 区域和身份

| 区域 | 生命周期 |
| --- | --- |
| `session` | 当前应用所有者的内存状态；服务重载继续共享，进程重启后清空。 |
| `durable` | 由应用所有者的文件适配器持久保存；按动作首次访问时加载，后续读取可使用宿主缓存。 |

身份来自服务为本次真实调用保留的包 ID 和动作 ID。同一身份跨绑定、动作版本、Node/Script 和服务重载共享状态；不同包或不同动作隔离。脚本不能通过参数、可读 `ctx.invocation`、自报 scope、绑定 ID 或文件路径选择别人的状态。多个字段需要一起更新时，应放在同一个 JSON 值中；没有跨键或跨动作事务。

应用输入引擎所有者显式挂载存储。命令行只有在 `--run-action` 同时指定布尔参数 `--with-action-state` 时才挂载相同的生产状态存储；durable 目录固定为所选 `--config` 同级的 `action-state`，构造存储不读写文件。未加参数的普通 CLI、没有挂载的 `ActionService::new`、独立运行时或没有真实服务作用域的脚本，调用时明确得到 `state_unavailable`，不会改用模块变量、临时文件或全局默认值。失效调用作用域为 `action_inactive`。此本地宿主协议不是针对恶意本地代码的安全沙盒。

```powershell
omnitrigger.exe --config D:\Demo\bindings.json --run-action D:\Demo\call.json --with-action-state
```

`--with-action-state` 不接受值、不能重复，不能单独使用或与检查、列目录、创建工程、安装运行时命令合用。不提供独立状态路径；每次 CLI 都创建新的服务会话，只有 durable 值可跨调用重新加载，session 状态不跨进程保留。

## 快照与缺值

`read(area, key)` 返回 `{revision, entry}`。`revision` 是宿主给出的不透明令牌，必须取自读取或 CAS 回执，不能自行生成、解析或复用到其他键、区域、动作或重启后的服务。

| `entry` | 含义 |
| --- | --- |
| `{present:false}` | 缺值，没有 `value` 字段。 |
| `{present:true,value:null}` | 已保存 JSON `null`。 |
| `{present:true,value:0}` | 已保存数字 `0`。 |
| `{present:true,value:""}` | 已保存空字符串。 |

这些状态互不等价。读取不会填入默认值；不要用 `value || 默认值` 判断存在性。输入值和读取结果按 JSON 复制，修改返回对象不会提交状态。

## 比较交换

`compareAndSwap(area, key, expectedRevision, next)` 在当前修订与令牌匹配时，提交一个完整 entry。返回 `{status:'committed',current}` 或 `{status:'conflict',current}`，其中 `current` 都是宿主确认的快照。冲突是正常回执，不是成功提交，也不是自动重试指令。

下面的 Node 示例由脚本显式写入数字 `0`，冲突后直接返回结果：

```js
/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  const before = await ctx.state.read('session', 'counter');
  const result = await ctx.state.compareAndSwap(
    'session', 'counter', before.revision, {present:true,value:0});
  return {status:result.status, current:result.current};
}
```

Script 使用相同参数，同步取得回执；此例显式写入持久区域的 JSON `null`：

```js
/** @type {ScriptMain} */
export function main(ctx) {
  const before = ctx.state.read('durable', 'choice');
  return ctx.state.compareAndSwap(
    'durable', 'choice', before.revision, {present:true,value:null});
}
```

删除使用 `next={present:false}`，不能同时带 `value`。删除和同值提交也推进修订；旧令牌不能凭值相同再次提交。服务重启产生新的令牌世代，durable 值可以重新加载，旧令牌仍会失效。

## 提交与错误边界

冲突后应根据业务重新决定后续动作，不自动重做 CAS、通知、输入或其他副作用。CAS 只协调状态；它与 `ctx.actions.stopOtherInstances`、桌面操作、通知或外部程序之间没有事务，不能回滚这些操作。

错误带 `code`、`method` 与 `effectState`。明确未开始的校验、无存储或提交前取消为 `not_started`。CAS 已派发后发生超时、断连、无效回执或不能确认的异常，保守返回 `unknown`；它不证明没有提交。替换文件完成后再收到取消，不能声称已经回滚。Node 返回时若仍有未 `await` 的状态调用，宿主以 `pending_state_call` 拒绝成功收尾；read 为 `not_started`，CAS 为 `unknown`。这不取消已经发生的提交。

durable 使用配置同级的 `action-state/<真实作用域哈希>.json`，键不参与路径。适配器检查文件格式、作用域与版本，通过锁、同目录临时文件及原子替换提交，确认成功后发布缓存与修订。损坏或不支持的文件报 `state_corrupt`，保留原文件，不将其当作缺值覆盖。存储不可用或失败分别保留 `state_unavailable`、`state_io` 等错误；适配器发现外部存储版本变化时报 `state_storage_changed`，清理相关缓存，下一次读取重新加载，不自动重做 CAS。

读取可来自缓存，不承诺每次重读磁盘。锁与版本检查约束正常适配器写入，不能隔离无视锁任意重写文件的外部程序。磁盘调用不能硬中断，时限检查不提供硬实时或断电后绝对耐久保证；退出时没有把旧会话快照统一回写的后台步骤。

## 范围限制

键为 1–64 个 UTF-8 字节，拒绝控制字符及无效 Unicode。每个区域最多 64 个键（包括本进程删除墓碑）；单个 JSON 值序列化后最多 16 KiB、深度最多 8、节点最多 2048，区域值文档最多 256 KiB。服务最多保留 128 个动作作用域。数值必须有限，整数须在 JavaScript 安全整数范围内；拒绝 `undefined`、循环引用、访问器和非 JSON 对象。

接口只接受 `session` / `durable`、键、修订和完整 entry；多余参数或身份字段为 `invalid_input`。配额或繁忙通过 `state_limit` / `state_busy` 明确返回，不静默删键或改写请求。
