# 错误与恢复语义

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
| `reload_failed`、`remote_error` | wire 预留类别；P1 没有重载或 OneBot 路由实现 |

正常方法拒绝可返回 err 响应；破坏连接状态的重复请求 ID、未知响应 ID、非法帧、超时等会使当前代次失败，不能假设收到错误后连接可继续使用。到期关闭连接且不复用，避免部分帧取消和迟到响应污染后续请求。自动重启预算为 0，显式 start 才重试启动。

副作用操作超时可能已执行；P1 本机管理等待超时也不能据此直接重复启停，应先查询 status。P2 动作路由将入队后超时/断线归类为 unknown_outcome；结果确定前不重试副作用。具体入队语义见 [动作指南](actions.md)。
