# ADR 0004：P3 本地包、配置事务与排他热重载

- 状态：已采纳；P3 已完成 Linux 本地验收
- 日期：2026-10-29
- 依据：AGENT.md、SKILL.md 的 P3 门槛，以及 ADR 0002/0003

## 决策

安装与启用分开。本地目录包复制到安装根同文件系统暂存目录，校验后原子发布到 `id/version`，禁止覆盖已有版本。拒绝链接、特殊文件、越界路径、过大包及声明的不支持目标平台；安装后保存 SHA256 清单用于损坏检测，不将摘要视为来源认证。未声明 target 的旧包保持兼容但不声称已认证平台。卸载必须拒绝被启用计划引用的版本，默认保留配置与数据。

清单新增可选 config_schema、config_version（默认1）、reload_policy（默认 exclusive）与 target。P3 只实现 exclusive；parallel_safe 返回 Unsupported。并行候选需要只读准备目录、数据写权限转移及激活协议，留待后续扩展，不以退化为排他方式掩盖语义差异。

插件配置为 TOML envelope（config_version/revision/values）。更新携带 expected_revision，经过离线 JSON Schema 与无副作用插件 prepare 校验，再同目录临时文件持久化并原子替换 desired，最后通知插件 commit。prepare/commit/abort/status 为 wire 1.2 请求；持久化或中间阶段失败时 abort 释放准备态。初始化携带可选不可变配置快照，SDK 提供动态已应用快照。旧插件不支持在线配置时明确返回 Unsupported，管理员可以持久化配置后排他重启，不能绕过 revision 校验。

秘密以 `{"$secret":"ENV_NAME"}` 保存，宿主仅在向插件提供快照时解析；管理读取仍返回引用，不返回解析值。错误仅报告字段路径与类别，不包含配置实例值。禁止远程 Schema 引用，避免校验器隐式访问网络或文件。私有文件权限与链接检查沿用本机管理信任边界；独立进程不是同用户文件沙箱。

磁盘期望修订与插件确认修订分开保存。prepare 失败不写入；落盘后 commit 失败保留 desired/applied 不一致，供状态查询和受控恢复。进程与文件之间没有原子事务。重启以 desired 为权威重新初始化；应用成功才确认 applied。禁止把脱敏占位符写回为秘密。

热重载先校验完整候选包、权限交集、依赖与配置，创建操作记录；默认拒绝有活动强依赖消费者的提供者替换，需先显式停用消费者。暂停旧代新投递，允许有期限排空，然后停止并回收直接子进程，确认释放端口，再启动候选。wire ≥1.2 必须等 SDK start 确认才判成功，旧插件保留握手兼容。候选失败则重新启动旧包与旧配置，仍使用新 generation，旧 BotRef 不恢复。恢复失败保持 Failed 并记录双重失败。业务磁盘数据和已发生外部副作用不回滚。排空溢出、超时及动作结果未知必须显式呈现，不自动重放。

管理操作记录 operation_id、调用主体标记 local-user、时间、方法、目标、阶段及安全结果摘要，不记录秘密或完整配置。实际主体认证沿用本机端点 UID/SID；日志不保留逐操作的 OS 用户标识或完整前后状态；另记录 before_version、expected_revision 和结果 revision（适用时）。耗时操作使用可查询状态；操作身份和取消清理必须在宿主所有权内，不随客户端断开丢失。管理变更串行，ConfigStore 用固定 OS 文件锁保护修订检查到原子发布。直接管理 API 的 request_key 仅在最多128条/256KiB记录保留期内去重，使用 SHA256 指纹绑定完整请求，同键不同载荷返回 Conflict；CLI 尚未暴露参数，不保证永久 exactly-once。宿主重启将未完成操作标为 interrupted，内存重载 journal 不恢复。completed 表示操作结束，reload 的 outcome 仍须区分 reloaded/rolled_back/failed。

## 验收

覆盖 A→B、B启动失败恢复A、恢复失败、旧代次路由、排空超时、端口租约释放、配置冲突/非法值/秘密引用/desired-applied不一致、依赖阻断及其他插件继续服务。保留 P1/P2 故障回归。更新 SDK rustdoc、Schema、配置/存储/重载/迁移指南与 CHANGELOG。平台证据只在实际执行后记录。
