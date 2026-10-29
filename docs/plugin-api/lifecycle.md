# 生命周期与失败恢复

P1/P2 启停与路由已交付，P3 配置事务、操作记录与排他热重载已通过 Linux 本地验收。运行状态快照包含 id、包版本、generation、状态与可选结构化 error。operation 的 running/终态描述管理任务，与插件进程状态不同；正在执行 reload 的操作不等于插件已完成初始化。

启动先校验完整计划，再绑定本机管理端点并按依赖顺序启动。单个插件启动失败不阻止无关插件启动，其依赖者不能正常启动。默认握手 10 秒，hello → negotiate → initialize → ready → activate 后宿主标记 active。SDK 在 activate 后运行 `Plugin::start`，wire minor >= 2 时宿主再调用 lifecycle.started 等待 SDK start 成功，失败触发启动失败或 reload 恢复；旧 minor < 2 仅握手兼容，业务注册可能尚未完成，须由 readiness/自定义探针确认。SDK 的业务 callbacks 在 start 完成后才调度，初始化不能等待依赖本插件业务回调的循环调用链。

每次真正尝试进程启动都分配新 generation，宿主生命周期内单调递增；instance_id 由宿主绑定，不能由插件自报。active 实例再次 start 返回 conflict。崩溃、超时或协议失败进入 failed，默认不自动重启；显式 start、reload 和失败恢复均须遵守依赖条件。宿主重启不承诺 generation 跨运行持久单调。

正常停止发送 control shutdown，默认给予 2 秒宽限，必要时 kill 并等待直接子进程退出。pending 请求得到取消或连接错误；已发出的动作可能只能报告 unknown_outcome，不自动重试。整体退出按逆拓扑停止；手动 stop 不自动级联，先停止活动强依赖消费者。BotRef、事件订阅与服务注册均属于当前实例，停止或失败时撤销，新启动不继承。旧 BotRef 不会自动转向新进程。

插件工作目录是包目录，业务 data_dir 为 `DATA/plugins/ID/data`，由 initialize 提供；不要根据 cwd 猜测数据根。宿主清空子进程继承环境，仅转发基础平台执行变量；P3 环境秘密由宿主解析后通过配置快照提供，不等同于把完整宿主环境转发给插件。stop、disable、uninstall 都不隐式删除业务数据，详见[存储](storage.md)。

宿主清理范围是直接启动的子进程，尚无进程树/作业对象监管。SDK 追踪自身回调任务，插件自己 spawn 的任务和子进程仍须保留所有权，绑定取消并在 stop 中 join/回收。独立进程和 kill_on_drop 不保证端口、文件锁或孙进程资源都已释放。

P3 reload 在 exclusive 策略下先检查候选，再停止旧代新投递并请求 `lifecycle.drain`。SDK 立即进入 draining，拒绝新业务，尚未执行的业务队列取消或丢弃，已排队但未执行的请求返回 cancelled，事件丢弃；已运行 callbacks（含 maintenance/start）有界等待，IPC reader 继续处理 pending 响应。成功返回 `{"drained":true}`，超时返回 timeout 且保持关闭新业务。SDK drain_timeout 默认 30 秒，宿主请求期限默认 5 秒、最大 30 秒；超时仍进入 stop/kill，不重试或重放。插件自建任务由 stop 负责取消和回收。`parallel_safe` 明确 unsupported。

候选启动失败尝试旧包与旧配置恢复，恢复启动仍是新 generation；业务磁盘与外部副作用不回滚。默认拒绝替换有活动强依赖消费者的提供者。完整阶段、双重失败与操作查询见[热重载](hot-reload.md)。

P3 配置 prepare/commit 在 SDK 专用串行 maintenance 队列处理，可以与事件、动作、服务回调并发。插件须同步共享业务状态；prepare 只验证，不产生副作用，commit 成功后 SDK 才替换动态 applied 快照。提交失败保留旧 applied，但宿主可能已经持久化 desired。配置状态与恢复语义见[配置](configuration.md)。

取消管理客户端等待不会转移实例或管理操作所有权。P3 耗时变更返回 operation id，宿主继续执行并保留最近操作供查询；重启把日志中 pending 操作标为 interrupted。旧 start/stop/call 仍等待结果，传输超时可能发生在操作已生效之后。重试前查询实际状态；只有处于保留范围内、使用同一 request_key 的管理请求具备去重保障。
