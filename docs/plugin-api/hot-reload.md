# 排他热重载

P3 已完成 Linux 本地验收，以下阶段与失败边界对应 ADR 0004 和当前实现。P3 仅支持清单 `reload_policy = "exclusive"`；`parallel_safe` 返回 unsupported。独占端口和可写数据目录不会同时交给新旧两代插件。

安装新版本与替换运行实例是两步。先将候选包安装到 `ID/VERSION` 并查询安装操作终态，再请求 reload。候选预检包括完整包摘要、平台、依赖、权限与配置；不满足条件应在停止旧实例前失败。当前版本的 grants 不会因为新清单声明更多能力而自动扩大，升级所需额外权限应通过明确的管理授权处理。

默认拒绝替换有活动强依赖消费者的提供者。先显式停用消费者，替换提供者，再恢复消费者；不要依赖宿主悄悄重启整个依赖图。弱依赖和服务调用仍须处理短暂不可用。修改启用计划、配置或运行实例的管理变更串行执行，并用 revision 和可选幂等键约束重复提交。

排他替换依次暂停旧代新投递、进行有期限排空、停止并回收直接子进程、确认资源释放、启动候选。SDK drain 拒绝新的 IPC 业务，取消或丢弃未执行队列并等待已运行回调；插件自行创建的外部入口、任务及派生进程须自行在 stop 中关闭、取消和 join。它不能制造进程与外部系统之间的事务。队列溢出、排空超时、停止超时和动作结果未知均须显式呈现，宿主不自动重放事件、动作或服务调用。

候选协商 wire minor >= 2 时，宿主还通过 lifecycle.started 等待 SDK start callback 成功，再完成启动；旧 minor < 2 仅握手兼容，仍需业务探针。旧进程停止后候选可能启动失败。恢复尝试重新启动旧包与旧配置，但分配新的 generation；旧 BotRef 无论新版本成功还是旧版本恢复都已失效。调用者须使用新事件携带的 BotRef 或重新获取注册信息，不能缓存旧路由后自动转发到新代。候选和恢复都失败时保留 Failed，操作结果须能区分原始失败与恢复失败。

恢复覆盖可执行包与初始化配置，不回滚插件业务数据、数据库模式、已经发送的消息或外部副作用。插件的迁移逻辑应在写入前判断旧版本是否仍可恢复。宿主只能直接管理已跟踪的子进程；插件自行派生而未回收的进程可能继续持有端口。为独占资源设计明确的关闭、join 与回收流程，不能将宿主强杀当作业务提交协议。

```sh
twintail-host plugins reload example-plugin 1.1.0 --data-dir ./data
twintail-host operations get OPERATION_ID --data-dir ./data
twintail-host plugins status example-plugin --data-dir ./data
```

reload 返回 `{"status":"ok","value":{"id":"op-…","state":"running"}}`；应轮询操作终态及目标状态后再恢复消费者。操作记录的终态为 completed、failed 或 interrupted；completed 的 reload 结果仍可能是 rolled_back，须查看 result.outcome，不能据 completed 判断候选已生效。plugins status 的 reload 诊断包含 generation 及候选/恢复结果。管理客户端断开不取消宿主已接管的操作。直接管理请求可提供非空、至多 128 字节且仅含 ASCII 字母数字及 `._-` 的 request_key，在最近 128 条保留操作范围内去重；当前 CLI 不提供该选项，不能假设命令重试天然幂等。同一键仅用于同一提交的重试，不用于提交改变后的配置或版本。宿主重启后日志中未完成操作标为 interrupted，须核对实际实例、启用计划和 desired/applied 后再继续。

在线配置 commit 与包 reload 是不同操作。配置落盘后 commit 失败可能留下 desired/applied 不一致；重启初始化采用期望配置，排他替换失败时恢复则采用该操作保存的旧快照。先查看[配置状态](configuration.md)，避免把“进程恢复成功”误认为所有期望配置已应用。升级前操作步骤见[迁移指南](migration.md)。

运行状态的最新 `reload` 结果含 id、outcome、state、generation、error、drain_error，双重失败另含 rollback_error。outcome 为 reloaded、rolled_back 或 failed。drain_error 为 null 表示无旧实例或成功排空；超时为 timeout，明确拒绝为 unsupported，连接关闭为 plugin_exited，队列拒绝为 overloaded。排空失败后仍执行 stop/kill，不能仅凭候选成功推断旧代排空完整。管理操作日志只保留结果摘要；详细诊断查看 plugins status。

core 的 reload journal 仅保存在每个运行 slot 内存中：最多一条执行中记录及最新 status.reload，不是磁盘恢复日志。DATA/operations.json 是独立管理日志，宿主重启后将未完成操作标为 interrupted；它不会自动重放内存 reload 的剩余阶段。

幂等键也绑定完整请求的 SHA256 指纹，同键不同载荷返回 Conflict；日志仅保存指纹，不保存配置值。操作还提供适用的 `before_version`、`expected_revision` 和结果 `revision`，这些字段不是完整状态历史。
