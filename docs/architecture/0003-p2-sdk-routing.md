# ADR 0003：P2 Rust SDK 与事件、动作、服务路由

- 状态：已实施，Linux 本地验收通过；平台证据见 [兼容性记录](../compatibility/README.md)
- 日期：2026-10-29
- 范围：P2；在 [ADR 0002](0002-p1-plugin-host.md) 的独立进程与 IPC 基线上增加 SDK 和业务路由

## 背景与边界

P1 已建立进程生命周期、握手、权限与本机管理。P2 要让无 QQ 凭据的 mock 适配器发布事件，echo 插件处理后调用 mock 动作，并能查询动作记录。宿主只负责通用路由，不依赖 echo、mock 或未来 SnowLuma 的实现。Rust SDK 依赖 `twintail-protocol`，插件依赖 SDK，宿主不链接插件业务代码；IPC 不传递 Rust ABI。

配置修订与热重载事务仍属 P3，真实 QQ、OneBot 和 SnowLuma 接入仍属 P4。本轮实例代次隔离不代表已经实现热重载。mock 闭环也不代表真实发送消息或外部兼容性已经验证。

## wire 与权限决策

wire 升至 `1.1`，主版本仍为 1。新增可选路由载荷与 `unknown_outcome` 错误码；协商按既有规则选择双方支持的 minor。P1 方法不受影响；旧宿主不支持的 P2 方法返回 `unsupported`。插件若必须依赖 P2，应将相应能力列为必需能力，不能仅以 major=1 推断路由可用。

以下请求通过请求/响应信封发送，载荷以 `crates/twintail-protocol/src/routing.rs` 为契约：

| 插件 → 宿主方法 | payload | 必需授予能力 |
| --- | --- | --- |
| `bots.register` | `RegisterBot` | `bots.register` |
| `events.subscribe` | `Subscribe` | `events.subscribe` |
| `events.publish` | `PublishEvent` | `events.publish` |
| `actions.call` | `ActionCall` | `actions.call` |
| `services.register` | `ServiceRegistration` | `services.provide` |
| `services.call` | `ServiceCall` | `services.call` |

能力仍取清单申请与宿主 grants 的精确交集，默认拒绝；无通配、前缀或隐含授权。以上是方法级能力，不宣称已有按账号、事件或动作名称细分的 ACL。宿主依据管道绑定调用方身份与 generation，不信任 payload 中自报的来源身份。

| 宿主 → 插件方法 | 信封 Body | payload |
| --- | --- | --- |
| `events.deliver` | `Event` | `EventEnvelope` |
| `actions.handle` | `Request` | `ActionCall` |
| `services.handle` | `Request` | `ServiceCall` |

事件没有逐条处理完成响应。动作与服务由目标以原 request_id 返回 `Response`；宿主负责维护各连接的相关性，不能将一条连接的 request_id 当成全局身份。

## 注册、身份与代次

机器人通过 `RegisterBot { self_id }` 注册在调用进程的宿主实例身份下。返回的 `BotRef` 包含宿主分配的 `adapter_instance_id` 与字符串 `self_id`（非空、至多 128 字节，不含控制字符或首尾空白；可保留非数字协议标识）。实例身份包含代次语义；业务插件须使用事件携带或注册返回的 BotRef，不能只凭 QQ 号、稳定 plugin id 或自造实例字符串选目标。

订阅、机器人与服务注册随实例退出移除。来源代次失效后不能继续注册或发布。动作根据 BotRef 定位适配器；服务先解析明确 provider 的当前注册，然后固定目标代次的发送端。一个在途请求不能在目标退出后重新查找同名插件并发送给新代。旧代响应只允许完成原调用，不可成为新代请求的响应。宿主保留 P1 的有界超时与故障清理规则。

## 事件投递与背压

`Subscribe.event_types` 替换调用者的完整精确事件类型集合，空数组取消全部订阅，不展开通配符。发布方只能为本代已注册的 self_id 发布事件。宿主填写 event_id、BotRef 与接收时间；protocol、event_type、payload 保留协议命名空间、消息段及未知扩展 JSON。

每个订阅实例具有独立、有界事件队列，与控制和响应队列分离。fan-out 使用有界准入，慢订阅者队列满时丢弃该订阅者的本次投递，不无限阻塞其他订阅者。发布响应 `DeliveryReport { delivered, dropped }` 统计队列准入；delivered 不代表插件已开始处理、业务成功或持久化。

投递为内存中尽力而为，不提供持久重放、exactly-once 或崩溃恢复。event_id 是宿主本地关联标识，不是上游去重键。顺序仅能在对应入队与读取路径上保持，业务 handler 并发完成顺序不保证；需要业务顺序或幂等时由插件显式处理。

## 动作与结果未知

`ActionCall` 包含 bot、action、params；调用 deadline 来自现有 RPC 超时机制，P2 载荷没有独立 deadline 字段。适配器接收 `actions.handle`，返回协议 JSON 或结构化错误，扩展参数与结果不由宿主裁剪。

宿主与 SDK 均不自动重试动作或服务调用。目标离线、权限不足等能够在派发前确定的失败使用对应错误。动作派发后发生超时或断线，副作用可能已经执行，须表达 `unknown_outcome`；调用者应查询业务状态或自行进行有依据的幂等恢复，不能把未知结果当作未执行后重新发送。适配器也应将远端结果不明映射为该错误。真实 OneBot 响应解释留到 P4。

## 命名服务

提供者以 `ServiceRegistration { name, version }` 注册 SemVer 接口版本。调用者显式发送 `ServiceCall { provider, service, version_req, method, params }`，provider 是稳定插件 id，version_req 是接口版本范围。宿主不自动挑选其他提供者，也不把服务接口版本当作插件包版本。

服务调用先校验提供者可用性、注册与 SemVer 范围，再固定当前代次。服务注册与调用分别需要 `services.provide` 和 `services.call`。清单强依赖控制启动顺序和运行依赖关系，与服务解析分工不同；调用者需要提供者在自身启动前就绪时应声明该强依赖。服务方法与参数由双方约定，通用宿主不解释 mock.emit 等业务方法。

## SDK 与验收要求

当前 SDK 要求协商 wire minor 至少为 1.1；低于此版本拒绝初始化。SDK 封装握手、激活与停止、请求关联、事件处理、动作与服务 handler，并限制队列和并发；stdout 仅用于协议，诊断写 stderr。处理宿主请求的 handler 必须能够同时调用宿主并接收结果，避免单读循环等待 handler 导致跨插件调用死锁。SDK 在 start 完成后才调度本插件业务回调；初始化不得等待依赖本插件回调的循环调用链。退出须取消并收拢 SDK 所有任务；插件自行创建的 detached 任务不在 SDK 追踪范围，插件须保留所有权并在 stop 中 join。

P2 验收至少覆盖：干净目录打包加载；mock.emit → echo 事件处理 → send_message → mock.actions 记录；权限拒绝；慢订阅者隔离与 dropped 准入报告；BotRef 来源约束；服务 provider 与 SemVer；退出撤销注册；固定代次目标与迟到响应；动作结果未知及无自动重试。具体实际命令和平台结果由验收记录登记，不以本段设计要求代替测试结果。

开发入口：[quickstart](../plugin-api/quickstart.md)、[事件](../plugin-api/events.md)、[动作](../plugin-api/actions.md)、[服务](../plugin-api/services.md)。
