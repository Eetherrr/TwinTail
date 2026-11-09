# 错误：区分拒绝、失败和结果未知

调用没有返回成功时，第一件事是判断“确定没执行”还是“可能已执行但响应丢了”。发送消息等有副作用的动作盲目重试可能重复执行。本页面向处理 SDK `Result`、CLI 错误和重载失败的作者与管理员。

前置知识：Rust 的 `Result<T, E>` 表示成功值或错误；RPC 是跨进程请求与响应。SDK 定义 `pub type Result<T = ()> = std::result::Result<T, RpcError>`。失败类型有 `code: ErrorCode` 与 `message: String` 两个字段；`RpcError::new(code: ErrorCode, message: impl Into<String>) -> Self` 可构造插件自己的错误，准确源码见 [protocol lib.rs](../../crates/twintail-protocol/src/lib.rs)。

## 先按 code 分类，不解析 message

```rust,ignore
match context.call_action(call).await {
    Ok(value) => { /* 按适配器约定处理 value */ }
    Err(error) if error.code == twintail_plugin_sdk::ErrorCode::UnknownOutcome => {
        // 记录待人工/业务查询确认的状态；不要自动重发。
    }
    Err(error) => { /* 按 error.code 处理；不要记录敏感 payload */ }
}
```

以上是回调内部片段，`call` 是已构造的 `ActionCall`；完整定义见[动作](actions.md)。可恢复的业务失败应在事件回调内消化，直接返回 `Err` 会使 SDK 事件 runtime 退出。

RPC 错误结构为 `{"code":"permission_denied","message":"capability host.health was not granted"}`，置于 response.outcome 的 err value。code 使用稳定 snake_case；message 为诊断文本，不是供程序解析的接口，**不得包含 token、秘密配置、完整插件输出或敏感消息内容**。协议错误示例与故障夹具也不得使用真实秘密。

| code | 含义与处理 |
| --- | --- |
| `incompatible_version` | wire 主版本或宿主 API 范围不兼容；核对版本后重启 |
| `permission_denied` | 必需能力未授予或调用未授权；核对申请和 grants |
| `offline` | 目标当前非 active；查询状态 |
| `timeout` | 握手、响应或写入超期；结果可能已发生，先查状态 |
| `unknown_outcome` | 动作入队后超时、断线或提供者报告结果未知；副作用可能已执行，禁止自动重试 |
| `cancelled` | 实例停止或调用取消 |
| `overloaded` | 有界队列、在途数或连接 ID 预算耗尽 |
| `invalid_config` | TOML、清单、路径或其他静态配置非法 |
| `plugin_exited` | 进程退出、无法执行或管道关闭 |
| `invalid_frame` | 帧长度、JSON 或传输错误 |
| `invalid_request` | 身份、顺序、request ID 或响应关联非法 |
| `dependency_failed` | 提供者缺失、版本不符、依赖环或运行时失效 |
| `not_found` | 配置/包资源或目标不存在 |
| `conflict` | 重复 id、重复 start 或仍有活动依赖者等状态冲突 |
| `unsupported` | 未支持的方法、能力或消息类型 |
| `reload_failed` | 插件替换失败；查看候选与恢复错误，以及实际运行版本 |
| `remote_error` | 上游服务返回错误；按适配器保留的结果结构判断，不盲目重试 |

## 错误是否使整个连接退役

正常方法拒绝可返回 err 响应；破坏连接状态的重复请求 ID、未知响应 ID、非法帧、超时等会使当前代次失败，不能假设收到错误后连接可继续使用。到期关闭连接且不复用，避免部分帧取消和迟到响应污染后续请求。自动重启预算为 0，显式 start 才重试启动。

副作用操作超时可能已执行；P1 本机管理等待超时也不能据此直接重复启停，应先查询 status。P2 动作路由将入队后超时/断线归类为 unknown_outcome；结果确定前不重试副作用。具体入队语义见 [动作指南](actions.md)。
