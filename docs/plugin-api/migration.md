# 迁移到 P3 包与配置管理

P3 已完成 Linux 本地验收，平台范围见项目兼容性记录。P1/P2 的源码示例和已打包二进制可以作为本地安装来源；P3 的版本安装目录、配置事务和排他替换不意味着既有部署已经自动迁移。

先为现有插件、宿主启用计划、配置与 `data/plugins/ID/` 业务数据制作可恢复副本，记录版本、grants、依赖及独占端口。备份必须由管理员自行完成；操作日志、包摘要及热重载恢复不替代业务数据备份。

将可执行文件与 plugin.toml 放入独立来源目录，确保入口为包内相对路径。Windows 入口和文件名须包含 `.exe`。来源目录不能包含链接；若带 checksums.json，必须与全部内容一致，安装时重新生成。用新版本号发布改变后的包，同一 id/version 不覆盖。新清单字段参阅[清单](manifest.md)：旧包省略 config_version、reload_policy 时分别为 1 与 exclusive，省略 target 保持兼容但没有目标平台认证。

按[存储指南](storage.md)安装并等待成功，再显式启用相应版本和 grants。安装不启用；停用与卸载不删除配置和业务数据。不要同时保留两份指向同一插件的启用记录，也不要通过直接编辑已安装目录绕过摘要或版本检查。已有 host.toml 相对路径计划仍可加载；迁移到版本安装根后，先停用旧条目并等待完成，再启用已安装版本，管理命令会持久化新的相对路径。宿主不会自动复制旧包到 ID/VERSION。

将旧业务配置转换为 TOML envelope 的 values；revision 由宿主管理，config_version 由插件定义。先通过 `config get` 获得当前 revision，再 validate 和 update。把秘密明文替换为 `{"$secret":"ENV_NAME"}` 并在宿主启动环境设置相应变量；确认 Schema 描述解析后的值。管理 get 返回引用，不返回已解析秘密，不能把脱敏显示当作可写回配置。详细提交与失败语义见[配置指南](configuration.md)。

使用在线 prepare/commit 的插件升级 SDK 与 wire 1.2 支持，实现无副作用 prepare、提交后应用与动态配置快照读取。不要只修改 hello 版本字符串就声称支持新控制方法。旧插件缺少在线支持时返回 unsupported；可先 stop、离线 config update（遵守 expected_revision）再 start；显式配置或有 Schema 的包仍需初始化确认，完全缺少 config.status 的旧插件应升级 SDK。只有无 Schema、revision 0 的默认空配置保留旧启停兼容。initialize 的历史配置不随 commit 改写，长期任务应在适当边界读取当前已应用快照。

执行包替换前先确认兼容的配置和数据格式，再停用活动强依赖消费者并等待操作完成。P3 只支持 exclusive；申请 parallel_safe 会失败。替换期间仍可能有在途业务产生副作用，结果不确定时不要自动重试。候选失败会尝试旧包和旧快照恢复，但所有重新启动均使用新 generation，旧 BotRef 必须丢弃。

迁移完成后分别检查 operation 终态、插件进程状态、desired/applied 修订、事件与动作业务闭环。包校验成功只证明包完整，operation 成功也不替代外部系统业务确认。若候选及恢复都失败，先保留错误与操作记录，核对包、配置和数据兼容性后受控恢复；不要反复重放发送动作。参阅[生命周期](lifecycle.md)及[热重载](hot-reload.md)。

## P6候选与离线状态迁移

P6仅准备本地候选，不外发或publish。升级前在维护窗口停止写入并备份独立数据根与包版本；恢复业务数据与恢复旧包是分别决策，热重载不撤销迁移或外部动作。tools/state已提供Unix离线snapshot/verify/restore，保留原字节，不转换Schema；恢复仅写新/空私有根，具体命令、端点占位与资源限制见[冷备恢复](../release/operations.md)。verify不证明环境引用可用或插件Schema通过，不把文件复制称作在线一致快照。候选发布约束见[发行入口](../release/README.md)。

适配器config_version保持1：新增transport枚举forward_ws（默认）/reverse_ws，旧url配置仍按forward_ws解释。reverse_ws时url省略或为空，reverse_bind默认127.0.0.1:17281且仅数字loopback，reverse_path默认/onebot/v11；其他配置限额继续适用。切换传输属于startup-only配置变化，使用stop→完整values CAS更新→start，不让旧正向连接与新监听同时工作。反向握手与监听规则见[安全边界](../architecture/security.md)。负责人已报告反向mock/SDK子进程闭环与重载重绑定通过；P6最终源码回归通过，P4真实验证范围不扩展。
