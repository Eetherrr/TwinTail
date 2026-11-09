# 配置：读取设置并可靠地更新

配置是管理员给插件的设置，例如服务地址、行为开关。宿主负责存盘和校验，插件拿到一份只读快照；不要在事件回调里直接覆盖宿主配置文件。本页适用于插件需要设置，或管理界面显示“期望”和“已应用”不一致时。

前置知识：会读 TOML/JSON；JSON Schema 是描述字段类型与限制的规则。“修订号 revision”用于防止两位管理员互相覆盖；CAS（比较并交换）表示只有旧修订仍匹配才接受写入。desired 是磁盘上期望的设置，applied 是插件最近确认已应用的设置。

## 先读取，再决定是否支持在线更新

[SDK 源码](../../crates/twintail-plugin-sdk/src/lib.rs)提供 `pub fn config(&self) -> Option<ConfigSnapshot>`。无初始化快照时返回 `None`；有值时得到独立副本，没有 RPC、没有单独读取权限，也不会自动监控磁盘文件。快照含 `config_version: u32`、`revision: u64`、`values: Value`，准确声明见 [configuration.rs](../../crates/twintail-protocol/src/configuration.rs)。读取示例：

```rust,ignore
if let Some(snapshot) = context.config() {
    let enabled = snapshot.values.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    // 使用 enabled；不要打印整个 snapshot，它可能含已解析的秘密。
}
```

本页描述配置存储、校验与在线更新契约；管理命令见下文，热重载恢复见[热重载](hot-reload.md)。

## 磁盘格式与四种版本概念

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

## Schema 校验与秘密引用

清单可用 `config_schema` 指定包内相对 JSON Schema 路径，`config_version` 默认为 1。Schema 使用 Draft 2020-12 离线校验；不允许 `$ref`、`$dynamicRef`、`$recursiveRef` 或 `$vocabulary`，包括本地引用。允许官方 Draft 2020-12 `$schema` 元数据声明。Schema 文件不得通过链接或路径越界逃出包目录，上限 1 MiB、深度 64、32768 节点；values 的 JSON 表示上限 1 MiB。用内联子 Schema 描述配置，避免依赖运行时下载或跨文件引用。

`{"$secret":"ENV_NAME"}` 是环境秘密引用，磁盘配置和管理查询保留此引用。宿主在校验与发送初始化/prepare 快照前，从自身进程环境解析为字符串；因此应在启动宿主前设置环境变量。Schema 应描述解析后的字符串形状。缺失或非法引用使校验失败；不要把缺失秘密改写为空字符串，也不要把脱敏占位符写回。校验错误只报告字段路径与固定原因，不能据此获得原始配置值。插件收到的是解析后的值，仍须避免将它写入日志、错误或自定义管理响应。

## 在线事务与失败后的状态

在线更新采用乐观并发：读取 revision，然后连同完整的新 values 提交 expected_revision。宿主先校验修订、秘密解析与 Schema，再让插件 prepare；prepare 必须无副作用，允许拒绝新快照。成功后宿主以同目录临时文件和原子替换持久化 desired，再发 commit，插件确认后标记 applied。prepare 被拒不改 desired；prepare 成功但宿主持久化失败时，宿主发送 `config.abort {"revision":N}` 清理候选。abort 匹配当前候选则清除，无候选时幂等成功，存在其他修订候选时返回 conflict。持久化后 commit 失败会保留 desired/applied 不一致，供管理查询与恢复。文件与进程之间没有共同的原子提交。

提交新修订时使用管理返回的当前 revision，遇到冲突重新读取并由调用者合并，不覆盖其他管理员的修改。已持久化 desired 是重启时的权威配置；旧进程仍可能运行旧 applied 快照。不要仅因更新操作报错就假设磁盘没有变化。运行态 SDK 使用 `Context::config() -> Option<ConfigSnapshot>` 读取当前 applied 的不可变副本，`initialize().config` 保持启动初值。插件实现 `prepare_config(Context, ConfigSnapshot)` 与 `commit_config(Context, ConfigSnapshot)`；二者默认返回 unsupported。prepare/commit 在专用串行 maintenance 队列运行，但可与事件、动作和服务回调并发，插件须同步共享状态。commit 成功后 SDK 才更新 applied；失败保留旧快照并清除 prepared，但 SDK 不撤销插件 commit 中已经发生的业务副作用，插件须自行管理。`config.status` 只返回 applied_revision/config_version，不返回 values，且在 start 完成后响应。

## 管理命令与离线更新

CLI 的 JSON 参数必须作为一个 shell 参数传入：

```sh
twintail-host config get example-plugin --data-dir ./data
twintail-host config validate example-plugin '{"endpoint":"http://127.0.0.1:9000","token":{"$secret":"TWINTAIL_TOKEN"}}' --data-dir ./data
twintail-host config update example-plugin 3 '{"endpoint":"http://127.0.0.1:9001","token":{"$secret":"TWINTAIL_TOKEN"}}' --data-dir ./data
```

get 返回引用快照与 desired/applied 状态。update 返回可查询的 operation，使用 `operations get OPERATION_ID` 跟踪完成，不能将 running 当作成功。上述语法已按当前 CLI 源码核对，运行验收结果另见项目记录。所有管理命令连接正在运行的宿主，只接受 --data-dir；--install-dir 在宿主启动时指定。

## 文件权限、锁与旧版兼容

新配置文件在 Unix 上使用 0600、目录使用 0700；Windows 继承私有宿主数据根 ACL。同目录临时文件先同步再原子替换，Unix 另同步目录；原子文件替换仍不能覆盖进程提交失败。persist/mark_applied 同时取得父目录 `.config-locks/<SHA256(namespace basename)>.lock` 稳定外部门闩及内部 `config.lock`，保护修订检查与替换，锁竞争返回 conflict；锁文件保持 0600，进程退出释放锁。稳定外部门闩不随插件移除而删除；只有彻底移除完整命名空间时才删除其内部 `config.lock`，且始终持有外部门闩，避免重建锁竞态。升级到这一锁协议前停止旧宿主和独立配置写入者，不支持与仅使用旧内部锁的旧进程并发 purge。多个 ConfigStore 对同一修订的竞争写入只能一个成功，插件 prepare/commit 的进程生命周期仍由管理层串行协调。

在线 prepare/commit 使用 wire 1.2 扩展；SDK 为 P2 兼容仍接受协商 1.1 及更高 minor，这不保证旧宿主提供配置方法。active 插件缺少 prepare 支持时明确返回 unsupported，不落盘。可先 `plugins stop ID`，再用同一 `config update ID EXPECTED_REVISION JSON` 离线仅持久化 desired，最后 `plugins start ID`；每一步等待完成。启动显式配置或有 Schema 的包仍要求配置确认，未实现 config.status 的旧插件不会被标为 applied，不能把离线写入视作兼容性保证。仅无 Schema、revision 0 的默认空配置保留 legacy 免确认路径。配置格式升级应由插件明确实现转换或拒绝，不把 semver 升级等同于自动迁移。参阅[迁移指南](migration.md)。
