# IPC v1

插件 stdin/stdout 只用于 `4 字节无符号大端长度 + UTF-8 JSON`。长度只计算 JSON，合法范围为 1–8 MiB，先验证长度再分配。stdout 调试文本会破坏连接；stderr 的 P1 行为见[排障](troubleshooting.md)。

所有消息带 `protocol_version: {"major":1,"minor":2}`；不同 major 拒绝，未知可选字段可忽略。P3 使用 wire 1.2，SDK 保留接受 1.1 及更高 minor，配置/启动确认扩展要求双方支持 1.2；1.0/1.1 插件保留对应的基线握手，新增配置和启动确认方法须双方支持。初始化可选携带 ConfigSnapshot，SDK 在 activate 后运行 start，宿主通过 lifecycle.started 确认应用初始化成功。配置方法和排空契约见 [配置](configuration.md) 与 [生命周期](lifecycle.md)。`kind` 为 `control`、`request`、`response` 或 `event`。control/request/event 使用 `method` 和 `payload`；request 另带 `request_id`，response 使用相同 `request_id` 和 `outcome`，没有独立 result/error 顶层字段：

```json
{"protocol_version":{"major":1,"minor":1},"kind":"request","request_id":"health:1","method":"host.health","payload":{}}
```

```json
{"protocol_version":{"major":1,"minor":1},"kind":"response","request_id":"health:1","outcome":{"status":"err","value":{"code":"permission_denied","message":"capability host.health was not granted"}}}
```

成功 outcome 为 `{"status":"ok","value":...}`。消息及载荷结构见 [wire schema](../../schemas/protocol-v1.json)。P2 通过 event `events.deliver` 携带 [事件封装](../../schemas/event-envelope-v1.json)。

## 握手

1. 插件发送 control `hello`，payload 含 `plugin_id`、包 `version` 和 `required_capabilities`。
2. 宿主校验 hello 身份、版本与必需能力集合与清单一致，并校验授予，发送 control `negotiate`，payload 为 `{"major":1,"minor":1}`。
3. 宿主发送 control `initialize`，payload 为 `instance_id`、`generation`、`granted_capabilities`、`data_dir`、`config_revision`。P1 revision 固定为 0，不表示已有配置事务。
4. 插件准备完成发送 control `ready`，payload 可为 `{}`；宿主发送 control `activate` 后进入 active。

宿主以启动的管道绑定身份，插件自报身份不能提升权限。默认整个握手期限 10 秒。收到 `initialize` 前不能预知宿主分配的 instance_id、generation 或数据目录；插件必须解析该消息。

## 运行态与限额

P1 插件可请求 `host.health`（需要同名能力），成功值包含 status、plugin_id、generation。P2 新增 `bots.register`、`events.subscribe`、`events.publish`、`actions.call`、`services.register` 和 `services.call`，对应载荷及权限见事件/动作/服务指南。未知方法返回 unsupported。插件发送 control `ping` 可获得 `pong`。宿主到插件的请求由 core 调用接口发起，CLI 提供同本机用户授权的 `plugins call ID METHOD JSON`；插件应按 request_id 回包。

| 资源 | 宿主上限/默认值 |
| --- | --- |
| 单帧 JSON | 8 MiB；读写均限制 |
| 宿主输出 control / response 队列 | 独立有界队列 8 / 32；control 优先；每项最多一帧 |
| 入站帧队列 / 实例命令队列 | 各 32 |
| 宿主到插件在途 RPC | 每实例 32 |
| 插件 request ID | 每连接累计最多 4096 个；不得重复 |
| request ID 格式 | 1–128 字节，ASCII 字母、数字、`-_.:` |
| 请求响应期限 | 默认 5 秒 |
| 管道写入期限 / 停止宽限 | 各默认 2 秒 |

响应 ID 必须对应当前在途请求；未知或重复响应使连接失败。请求到期、写入超时、协议错误或响应队列耗尽会终止当前连接与代次并清理 pending；部分帧读写取消后关闭连接，不复用可能失步的字节流。仅增加有界队列不构成完整进程内存或 CPU 限制，JSON 编码/解析仍有额外开销。

本机管理 IPC 与插件管道独立，使用相同信封、1 MiB 帧上限、网络读写默认 5 秒，每连接处理一次请求；生命周期操作等待有独立期限，以当前代码为准。操作等待超时不证明启停未执行，应先查询 status。

P2 增加独立的每实例事件队列（32），不与控制/响应队列共享。`events.publish` 返回 `DeliveryReport {delivered,dropped}`：表示入队数量，不是业务处理确认。动作与服务通过目标代次固定的有界转发通道；动作已入队后丢失结果返回 `unknown_outcome`，禁止自动重试。注册表在代次退役时清除，详见 [ADR 0003](../architecture/0003-p2-sdk-routing.md)。
