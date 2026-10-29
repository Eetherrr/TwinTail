# 插件配置

P3 已完成 Linux 本地验收，平台范围见项目兼容性记录。本页描述配置存储、校验与在线更新契约；管理命令见下文，热重载恢复见[热重载](hot-reload.md)。

每个插件的期望配置位于 `data/plugins/ID/config.toml`，使用 TOML envelope：

```toml
config_version = 1
revision = 3

[values]
endpoint = "http://127.0.0.1:9000"

[values.token]
"$secret" = "TWINTAIL_TOKEN"
```

`config_version` 是插件定义的配置格式版本，`revision` 是宿主维护的并发修订号；不要将二者与插件版本、wire minor 或运行代次混用。尚无配置文件时读取为 revision 0、values `{}`；这不表示空配置满足插件 Schema，启用时仍须校验。`config.applied.toml` 单独保存最近确认的 revision。状态报告 desired_revision 与可选 applied_revision，二者相等仅表示最近一次确认一致，不表示当前插件进程存活。

清单可用 `config_schema` 指定包内相对 JSON Schema 路径，`config_version` 默认为 1。Schema 使用 Draft 2020-12 离线校验；不允许 `$ref`、`$dynamicRef`、`$recursiveRef` 或 `$vocabulary`，包括本地引用。允许官方 Draft 2020-12 `$schema` 元数据声明。Schema 文件不得通过链接或路径越界逃出包目录，上限 1 MiB、深度 64、32768 节点；values 的 JSON 表示上限 1 MiB。用内联子 Schema 描述配置，避免依赖运行时下载或跨文件引用。

`{"$secret":"ENV_NAME"}` 是环境秘密引用，磁盘配置和管理查询保留此引用。宿主在校验与发送初始化/prepare 快照前，从自身进程环境解析为字符串；因此应在启动宿主前设置环境变量。Schema 应描述解析后的字符串形状。缺失或非法引用使校验失败；不要把缺失秘密改写为空字符串，也不要把脱敏占位符写回。校验错误只报告字段路径与固定原因，不能据此获得原始配置值。插件收到的是解析后的值，仍须避免将它写入日志、错误或自定义管理响应。

在线更新采用乐观并发：读取 revision，然后连同完整的新 values 提交 expected_revision。宿主先校验修订、秘密解析与 Schema，再让插件 prepare；prepare 必须无副作用，允许拒绝新快照。成功后宿主以同目录临时文件和原子替换持久化 desired，再发 commit，插件确认后标记 applied。prepare 被拒不改 desired；prepare 成功但宿主持久化失败时，宿主发送 `config.abort {"revision":N}` 清理候选。abort 匹配当前候选则清除，无候选时幂等成功，存在其他修订候选时返回 conflict。持久化后 commit 失败会保留 desired/applied 不一致，供管理查询与恢复。文件与进程之间没有共同的原子提交。

提交新修订时使用管理返回的当前 revision，遇到冲突重新读取并由调用者合并，不覆盖其他管理员的修改。已持久化 desired 是重启时的权威配置；旧进程仍可能运行旧 applied 快照。不要仅因更新操作报错就假设磁盘没有变化。运行态 SDK 使用 `Context::config() -> Option<ConfigSnapshot>` 读取当前 applied 的不可变副本，`initialize().config` 保持启动初值。插件实现 `prepare_config(Context, ConfigSnapshot)` 与 `commit_config(Context, ConfigSnapshot)`；二者默认返回 unsupported。prepare/commit 在专用串行 maintenance 队列运行，但可与事件、动作和服务回调并发，插件须同步共享状态。commit 成功后 SDK 才更新 applied；失败保留旧快照并清除 prepared，但 SDK 不撤销插件 commit 中已经发生的业务副作用，插件须自行管理。`config.status` 只返回 applied_revision/config_version，不返回 values，且在 start 完成后响应。

CLI 的 JSON 参数必须作为一个 shell 参数传入：

```sh
twintail-host config get example-plugin --data-dir ./data
twintail-host config validate example-plugin '{"endpoint":"http://127.0.0.1:9000","token":{"$secret":"TWINTAIL_TOKEN"}}' --data-dir ./data
twintail-host config update example-plugin 3 '{"endpoint":"http://127.0.0.1:9001","token":{"$secret":"TWINTAIL_TOKEN"}}' --data-dir ./data
```

get 返回引用快照与 desired/applied 状态。update 返回可查询的 operation，使用 `operations get OPERATION_ID` 跟踪完成，不能将 running 当作成功。上述语法已按当前 CLI 源码核对，运行验收结果另见项目记录。所有管理命令连接正在运行的宿主，只接受 --data-dir；--install-dir 在宿主启动时指定。

新配置文件在 Unix 上使用 0600、目录使用 0700；Windows 继承私有宿主数据根 ACL。同目录临时文件先同步再原子替换，Unix 另同步目录；原子文件替换仍不能覆盖进程提交失败。`config.lock` 使用 OS 文件锁保护 persist/mark_applied 的修订检查与替换，锁竞争返回 conflict；文件保持 0600 且不删除，进程退出释放锁。多个 ConfigStore 对同一修订的竞争写入只能一个成功，插件 prepare/commit 的进程生命周期仍由管理层串行协调。

在线 prepare/commit 使用 wire 1.2 扩展；SDK 为 P2 兼容仍接受协商 1.1 及更高 minor，这不保证旧宿主提供配置方法。active 插件缺少 prepare 支持时明确返回 unsupported，不落盘。可先 `plugins stop ID`，再用同一 `config update ID EXPECTED_REVISION JSON` 离线仅持久化 desired，最后 `plugins start ID`；每一步等待完成。启动显式配置或有 Schema 的包仍要求配置确认，未实现 config.status 的旧插件不会被标为 applied，不能把离线写入视作兼容性保证。仅无 Schema、revision 0 的默认空配置保留 legacy 免确认路径。配置格式升级应由插件明确实现转换或拒绝，不把 semver 升级等同于自动迁移。参阅[迁移指南](migration.md)。
