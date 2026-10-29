# 动作调用与结果未知

P2 已完成 Linux 本地验收。Rust SDK 的 `Context::call_action` 将 `ActionCall` 发送到宿主 `actions.call`，需要清单申请并由宿主 grants 授予同名能力。适配器接收宿主请求 `actions.handle`，SDK 转交 `Plugin::action`，返回任意 JSON 结果或 `RpcError`。方法与能力对照见 [ADR 0003](../architecture/0003-p2-sdk-routing.md)。

```rust,ignore
use serde_json::json;
use twintail_plugin_sdk::ActionCall;

let result = ctx.call_action(ActionCall {
    bot: event.bot.clone(),
    action: "send_msg".into(),
    params: json!({"message": event.payload["message"]}),
}).await?;
```

这里的 event 是 `Plugin::event` 收到的 EventEnvelope，ctx 是回调的 Context。动作名称和 params 由具体适配器约定；示例使用 OneBot v11 标准动作名 send_msg；mock 仅记录调用，真实传输由 adapter-snowluma 提供。可运行示例的准确参数见 [quickstart](quickstart.md) 与 echo-plugin 源码。

BotRef 包含 `adapter_instance_id` 和字符串 `self_id`，后者保留协议账号标识，也避免数字账号在 JSON/浏览器中损失精度。适配器通过 `Context::register_bot(RegisterBot { self_id })` 获取自身 BotRef，业务插件通常使用 event.bot。不能仅凭 QQ 号选择路由，也不能将稳定插件 id 填入 adapter_instance_id。宿主检查账号确已在对应实例注册，再把动作送到那个实例的固定代次发送端。

`actions.handle` 使用请求/响应信封，payload 仍为 ActionCall。SDK handler 成功返回的 JSON 原样回给调用者，宿主不解释远端协议 retcode。适配器负责区分协议失败和传输失败、保留所需诊断字段，同时避免将 token 或完整敏感消息写进错误文本。真实 OneBot status/retcode/data 的映射属于 P4。

动作载荷只有 bot、action、params；P2 没有动作特有的 deadline 字段，也没有通过 params 设置宿主超时的约定。请求受宿主与 SDK 的有界在途表和超时控制。不要向 payload 添加未经实现的 timeout 字段后假定它生效。

| 情况 | 调用者处理原则 |
| --- | --- |
| `permission_denied` | 核对清单 capabilities 与 host.toml grants 的精确交集 |
| 目标离线、账号不存在或队列拒绝 | 根据具体错误修正目标或负载；不要把所有失败归为远端失败 |
| `unknown_outcome` | 副作用可能已执行；查询业务状态或使用明确的幂等恢复方案 |
| 远端明确拒绝 | 按适配器返回的结构化错误处理，不凭诊断 message 分支 |

wire 1.1 的 `ErrorCode::UnknownOutcome` 序列化为 `unknown_outcome`。宿主已经派发动作后发生超时或断线，无法仅凭未收到响应断定动作没执行。适配器在外部系统发送后失去结果时也应返回该类别。发送消息、踢人等副作用动作直接重发可能执行两次。

**SDK 与宿主不自动重试动作。** 即使某个动作在业务上是幂等查询，也应由调用者显式决定重试范围与次数。调用者取消等待不代表撤销适配器或外部系统的副作用。目标退出后不会把在途请求重定向给同 id 的新代次；旧 BotRef 应视为已失效，重新从新事件或注册结果获取目标。

本轮 mock 只验证事件到动作记录的进程路由，不能证明消息已发到 QQ，也不能证明重载期间的业务连续性。配置与热重载仍为 P3，QQ/SnowLuma 为 P4。停止、协议诊断与 stdout 限制参阅[生命周期](lifecycle.md)和[排障](troubleshooting.md)。
