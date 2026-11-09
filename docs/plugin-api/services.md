# 服务：向另一个插件请求数据或能力

命名服务是插件之间约定的一套 JSON 问答。例如词库插件提供查词服务，回复插件调用它；双方不需要链接对方的 Rust 代码。它不同于[动作](actions.md)：动作按机器人 `BotRef` 路由，服务按提供者插件 ID 和服务名路由。

前置知识：理解 JSON 和 SemVer（`1.0.0` 是接口版本，`^1.0.0` 是调用者接受的版本范围）。准备把功能拆成多个插件时读本页；只做消息回复可以暂时跳过。

## 先注册，再调用

[Context 源码](../../crates/twintail-plugin-sdk/src/lib.rs)提供 `pub async fn register_service(&self, registration: ServiceRegistration) -> Result<Value>` 和 `pub async fn call_service(&self, call: ServiceCall) -> Result<Value>`。前者返回注册确认 JSON，后者返回提供者定义的 JSON，二者失败均为 `RpcError`。完整字段来自 [routing.rs](../../crates/twintail-protocol/src/routing.rs)。

命名服务让插件通过版本化 JSON 请求协作，宿主与调用者不链接提供者实现。Rust SDK 提供 `Context::register_service`、`Context::call_service` 与 `Plugin::service` 回调。线协议方法与权限见 [ADR 0003](../architecture/0003-p2-sdk-routing.md)。

提供者申请并获得 `services.provide`，在 `Plugin::start` 中注册：

```rust,ignore
use twintail_plugin_sdk::ServiceRegistration;

ctx.register_service(ServiceRegistration {
    name: "example.echo".into(),
    version: "1.0.0".into(),
}).await?;
```

宿主方法为 `services.register`，能力名为 `services.provide`，两者不同。注册表绑定真实进程身份与 generation，同一实例重复注册相同 name 会覆盖该服务的版本；退出时移除该代所有注册。服务接口版本是 SemVer，与清单中的插件包版本分开维护。

调用者申请并获得 `services.call`，显式填写提供者、服务与兼容范围：

```rust,ignore
use serde_json::json;
use twintail_plugin_sdk::ServiceCall;

let reply = ctx.call_service(ServiceCall {
    provider: "example-provider".into(),
    service: "example.echo".into(),
    version_req: "^1.0.0".into(),
    method: "echo".into(),
    params: json!({"text": "hello"}),
}).await?;
```

provider 是稳定插件 id，service 是该提供者下的精确服务名。宿主不在其他插件中自动寻找替代提供者；version_req 匹配已注册的服务接口版本，不匹配返回 `incompatible_version`。提供者离线返回 `offline`，服务未注册返回 `not_found`；非法名称或 SemVer 返回 `invalid_request`。错误 message 是诊断文本，不是稳定程序接口。

清单 `[dependencies]` 是插件包版本范围，用于启动顺序与生命周期依赖；服务 version_req 是接口范围，用于每次调用校验。两者不能互相替代。需要在自身 start 时调用提供者的插件，应声明强依赖，确保提供者已经启动并注册服务。清单强依赖还会影响停用与故障级联，详见[清单指南](manifest.md)。SDK 在本插件 start 完成后才调度业务回调；start 中的服务调用链不得再等待本插件回调，否则形成初始化循环。

宿主解析当前注册后固定该次调用的目标代次发送端，以 `services.handle` 请求将完整 ServiceCall 转发给提供者。SDK 调用 `Plugin::service(ctx, call)`，handler 根据 call.service 和 call.method 处理 params，返回 JSON 或 RpcError。params 和结果的 schema 由服务双方定义；接口发生破坏性变化时升级服务主版本，并为调用者说明迁移与错误行为。

### 提供者如何实现回调

下面放在已实现 `Plugin` 的类型中，与前面的注册名称对应。完整程序入口、Hello 与打包结构沿用[第一个插件](first-plugin.md)，清单及 grants 另外加入 `services.provide`。`params` 的类型和校验规则由服务作者负责；这个示例只原样返回，不隐式做配置或数据库写入。

```rust,ignore
async fn service(
    &self,
    _context: twintail_plugin_sdk::Context,
    call: twintail_plugin_sdk::ServiceCall,
) -> twintail_plugin_sdk::Result<serde_json::Value> {
    if call.service == "example.echo" && call.method == "echo" {
        Ok(call.params)
    } else {
        Err(twintail_plugin_sdk::RpcError::new(
            twintail_plugin_sdk::ErrorCode::Unsupported,
            "unknown service method",
        ))
    }
}
```

调用者收到的成功值就是 handler 返回的 JSON；返回 RpcError 仅拒绝本次请求，和 event 返回错误会使 runtime 退出的语义不同。调用方应按 error.code 区分离线、版本不符、过载与服务业务拒绝。

## 限制、超时与退出

服务名、method 等路由名称限制为非空、至多 128 字节的 ASCII 字母数字及 `._:-/`；version 最长 128 字节，version_req 最长 256 字节。宿主每实例注册数量、转发并发与队列均有上限，满载可能返回 `overloaded`。不要依赖无限注册或无限并发。

宿主与 SDK 不自动重试服务。超时或取消等待不表示服务副作用已撤销；服务自己应规定幂等键、状态查询及恢复语义。目标退出清理注册，不会把在途请求改送到同名新代次；旧代响应只能属于原调用。服务超时还可能触发目标连接退役，应检查实例状态，而不是无限重试。

mock-adapter 示例注册 `mock.inspect` 服务 1.0.0，service handler 将 params 原样返回，适合测试版本化服务调用。[quickstart](quickstart.md) 中的 `mock.emit` 和 `mock.actions` 则是自定义 request 入口，分别注入消息与查询动作记录。管理员的 `plugins call` 通过独立本机管理身份调用这些请求，不能据此推断普通插件已获得 services.call；插件之间仍使用本页的受能力检查路由。

服务路由本身不构成配置事务或热重载的数据迁移支持。秘密不得进入服务诊断、样例或日志；stdout 始终仅承载帧协议。
