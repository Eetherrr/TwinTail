# 事件：订阅收到的消息与通知

事件是适配器发给业务插件的一条通知，例如“收到私聊消息”。业务插件先订阅类型，宿主才会投递匹配通知；适配器则负责注册账号、发布通知。本页分别说明这两个角色，不要求业务插件实现网络连接。

前置知识：认识异步函数和 JSON；建议先跑[快速开始](quickstart.md)。`payload` 是原始协议内容，`EventEnvelope` 是宿主加上来源和时间后的外层包装。“尽力投递”表示可能丢失，不是可靠消息队列。

## 订阅：业务插件最常用的入口

[Context 源码](../../crates/twintail-plugin-sdk/src/lib.rs)中的实际签名是 `pub async fn subscribe(&self, subscription: Subscribe) -> Result<Value>`。成功返回宿主 JSON 确认，失败返回 `RpcError`；权限不足为 `PermissionDenied`。回调签名与完整程序见[SDK 参考](sdk-reference.md#plugin回调)。

本文对应 wire 1.2（路由载荷沿用 1.1）；设计依据见 [ADR 0003](../architecture/0003-p2-sdk-routing.md)，真实部署范围见[兼容矩阵](../compatibility/snowluma.md)。

插件在清单申请 `events.subscribe`，并由 host.toml grants 显式授予后，调用 `Context::subscribe(Subscribe { event_types })`。订阅采用 event_type 精确匹配，不支持通配符；每次调用替换完整集合，空数组取消全部订阅。protocol 是事件内容中的命名空间，不是另一个自动匹配条件。

```rust,ignore
use twintail_plugin_sdk::Subscribe;

ctx.subscribe(Subscribe {
    event_types: vec!["message".into()],
}).await?;
```

上例中的 `ctx` 是 SDK 传入的 `Context`，应在 `Plugin::start` 中进行订阅。收到事件时 SDK 调用 `Plugin::event(ctx, event)`，event 为 `EventEnvelope`。适配器必须同时申请并获得 `bots.register`、`events.publish` 才能注册账号、发布事件：

```rust,ignore
use serde_json::json;
use twintail_plugin_sdk::{PublishEvent, RegisterBot};

let bot = ctx.register_bot(RegisterBot { self_id: "10001".into() }).await?;
let report = ctx.publish(PublishEvent {
    self_id: bot.self_id,
    protocol: "mock.v1".into(),
    event_type: "message".into(),
    payload: json!({
        "message": [{"type": "text", "data": {"text": "hello"}}],
        "extension": {"source": "local-demo"}
    }),
}).await?;
```

## 发布方身份与事件字段

发布方只能为本进程、本代次注册过的 self_id 发布。宿主根据真实进程身份生成 `EventEnvelope.bot`，不能以 payload 中自报的 plugin_id 或 BotRef 冒充其他适配器。BotRef 的 adapter_instance_id 包含宿主实例及代次语义；后续动作应直接使用 event.bot，详见[动作指南](actions.md)。

| EventEnvelope 字段 | 语义 |
| --- | --- |
| `event_id` | 宿主本地投递关联标识，不能替代上游消息去重键 |
| `bot` | 经过宿主身份绑定的适配器实例与 self_id |
| `received_at_ms` | 宿主接收时间，Unix epoch 毫秒 |
| `protocol` | 发布者的协议命名空间，例如 mock.v1 |
| `event_type` | 精确订阅匹配类型 |
| `payload` | 原始协议 JSON，保留消息段、未知类型和扩展字段 |

## 入队不等于处理完成

线上投递使用 `Body::Event`，方法为 `events.deliver`，payload 为 EventEnvelope；它没有逐事件响应。SDK handler 返回成功也不构成发送方能够观察的业务完成确认。

`publish` 返回 `DeliveryReport { delivered, dropped }`。delivered 仅统计成功进入各订阅者有界队列的数量；dropped 表示匹配订阅者在准入时未能接收。没有订阅者时不意味着业务成功。事件队列与控制、响应队列分离；慢订阅者队列满会丢弃其本次投递，其他订阅者仍可接收。不要把 report 当作全体 handler 完成、持久化或送达上游用户的证明。

事件是内存中尽力投递，进程崩溃或停止可能丢失已经准入的事件，没有持久重放或 exactly-once 承诺。订阅和机器人注册随实例退出清除，重新启动后由 start 重新建立。处理并发时完成顺序可能不同；业务要求会话顺序时自行串行化，需要去重时使用来源消息 id 和业务键。框架 event_id 不能消除上游断线重连产生的重复。

## 限额和回调错误

宿主每实例事件队列当前容量为 32，机器人、订阅和服务注册各最多 128；帧 JSON 硬上限仍为 8 MiB，路由封装后超限返回 invalid_request。事件类型与 protocol 限非空、至多 128 字节的 ASCII 字母数字及 `._:-/`。

SDK 队列同样有界。当前 SDK 的 event handler 返回错误会终止本代 runtime；可恢复的业务失败应由 handler 明确处理。事件 handler 避免长时间同步阻塞；诊断写 stderr，stdout 专用于协议。插件停止时通过 `Context::cancelled().await` 或 `is_cancelled()` 配合退出，不在停止后继续发布。配置事务与热重载恢复分别见[配置](configuration.md)与[热重载](hot-reload.md)；当前不暂存迁移事件，代次隔离不构成无损重载保证。
