# TwinTail 项目执行约定

本文适用于整个仓库。架构与开发阶段的权威设计入口为 [SKILL.md](SKILL.md)，变更历史为 [CHANGELOG.md](CHANGELOG.md)。文档中的设计不代表功能已经实现。

## 项目现状与目标

**P3 管理、配置与热重载已完成 Linux 本地验收**：Rust 2024 workspace 包含 protocol/core/host/SDK 和 echo/mock 示例，工具链与最低支持版本为 Rust 1.99.1。已有 wire 1.2、版本包管理、配置 Schema/CAS/秘密引用及 prepare/commit/abort、desired/applied、可查询持久操作、排他热重载与失败恢复。继续保留静态权限/依赖、直接子进程监管、有界事件/动作/服务路由和 BotRef。P4 OneBot 类型库和 SnowLuma 正向 Universal WS 适配器已完成 Linux 本地验收及指定真实部署范围验证；P5 WebUI 与独立管理插件已完成 Linux 本地验收。P6 已实现 loopback 反向 Universal WS、候选打包校验、冷备份恢复和插件契约/有界负载工具，Linux 候选证据见发行文档；反向真实 SnowLuma、split/HTTP 与 Windows/arm64 运行未验证。项目采用 MIT 许可证，GitHub 发行已获授权。范围见 [ADR 0007](docs/architecture/0007-p6-release-hardening.md)。Windows 运行仍待目标 CI。P0–P2 历史见 ADR 0001–0003，P3 契约见 ADR 0004，P4 契约见 [ADR 0005](docs/architecture/0005-p4-onebot-snowluma.md)，运行步骤和证据见[适配器指南](docs/plugin-api/adapters.md)与[真实兼容矩阵](docs/compatibility/snowluma.md)。

目标是 Rust QQ Bot 框架，首选 OneBot v11，首个接入实现为 SnowLuma；支持插件管理、配置、热重载、WebUI 与插件开发 API 文档。

## 开始任务

1. 阅读本文、SKILL.md、CHANGELOG.md 及相关现有代码；检查工作区状态，不覆盖用户未提交改动。
2. 确定任务所属阶段、依赖、交付物及验收条件。默认沿 P0→P6 推进；调整阶段顺序需说明原因并同步设计。
3. 区分已实现能力、设计目标和外部兼容性假设；先验证接口再依赖，不根据产品宣传宣称已完成集成。
4. 协议、权限、依赖方向、热重载和存储兼容的重大变更记录到 `docs/architecture/` 的 ADR，并同步 SKILL.md；常规实现按现有设计推进。

## 必须保持的架构边界

- 最小宿主只负责启动、监管、路由、授权、配置根、存储命名空间与管理控制面。SnowLuma 适配、业务逻辑和 Web 管理服务器作为独立插件。
- 默认使用独立进程插件与版本化 IPC，不引入可热卸载 Rust 原生动态库。Rust SDK 是协议的便利封装，不能依赖跨进程 Rust ABI。
- 子进程 stdin/stdout 专用于长度前缀协议，stderr 专用于诊断；P1 只用固定缓冲读取并丢弃，尚无日志展示或完整速率/CPU 限制。插件不得向 stdout 输出调试文本。控制消息、响应与业务事件分别限流，所有队列有界。
- 依赖方向为 wire 类型→SDK/核心→宿主或插件。宿主不可依赖某个插件实现；跨插件调用通过版本化服务与权限检查。
- SnowLuma 独立部署；adapter-snowluma 连接其 OneBot 服务，不把 QQ 登录实现、SnowLuma 私有管理接口或其 WebUI 嵌入核心。
- 权限默认拒绝，插件清单申请不等于宿主授权；独立进程不等于沙箱。MVP 仅运行受信任插件。
- 事件保留 OneBot 扩展字段和消息段；动作以适配器实例及 self_id 定位。发送超时可能已执行，非幂等动作不得自动重试。

## 生命周期、配置与数据

- 生命周期由宿主统一协调；每次替换使用新 generation，旧代回包只能交还原调用方，不得重新路由给新代实例。
- 热重载必须有校验、ready、排空、超时、切换和恢复失败的明确结果；适配器与监听端口的插件默认排他替换。不得宣称无损或零停机。
- parallel_safe 候选在激活前不得执行外部副作用，不能与旧代并发写共享私有数据；准备阶段使用只读快照或代次暂存目录，写入权限在提交时转移。无法满足此约束的插件必须使用 exclusive。
- 任务组、连接、子进程、服务注册和待响应请求均须随实例退出清理；限制崩溃重启预算；P1 自动重启预算为 0，failed 需显式 start。P1 仅监管直接子进程，插件禁止派生守护子进程。
- 配置具有 schema、config_version、revision，更新经过校验、准备、原子落盘和运行态确认；明确 desired/applied 不一致和恢复行为。
- 秘密使用引用或受控环境注入，禁止进入源码、样例、日志、浏览器响应或 CHANGELOG。密钥的脱敏占位符不得被写回为真实值。
- 包版本与数据目录分离；卸载默认保留配置与数据。破坏性数据迁移不属于通用在线热重载，须单独备份、停机并提供迁移说明。
- WebUI 调用宿主管理服务，不直接写文件；保留 CLI 与本机管理通道作为恢复入口。管理 HTTP 默认 loopback，必须认证、鉴权并防护 CSRF 与不可信 Origin。

## 文档与接口交付

- 新增或修改公共 API 时同步 rustdoc、wire schema、示例与 `docs/plugin-api/`；HTTP 管理接口同步 OpenAPI。
- P2 建立可运行 echo-plugin 和 quickstart；P4 补适配器作者指南与真实 SnowLuma 兼容矩阵；P5 补 Web API 与 UI 操作说明。每个阶段文档随功能交付，不推迟到最终发布。
- 插件文档至少覆盖 manifest、版本协商、生命周期、事件/动作/服务、错误、权限、配置、持久化、热重载、迁移和调试。
- 对外版本破坏性变化必须说明兼容范围、迁移路径与错误行为；不以注释替代用户可用的开发指南。

## 验证与完成条件

代码实施后按实际影响执行：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

前端存在后执行锁文件约束的安装、类型检查、构建和关键交互测试。新增生命周期/路由行为要覆盖超时、崩溃、权限、背压与旧代消息；配置变更覆盖修订冲突、非法配置和恢复。协议默认使用脱敏夹具和 mock，真实 QQ 集成不进入无凭据 CI。只修改文档时检查链接、术语与阶段一致性，无需为文档新增测试。

一次任务完成应同时满足：功能与任务验收匹配、必要检查通过、API/示例/设计与实现一致、CHANGELOG 已更新。汇报实际验证命令与结果，注明外部兼容性未验证和未完成范围；不得将规划条目描述为实现成果。

## 所有项目改动必须有 CHANGELOG

任何纳入项目的改动（代码、文档、配置、依赖、工具链、测试、构建、生成物等）都必须在同一提交/交付中更新 CHANGELOG.md。一个任务的相关改动可合并为一条，无需逐文件记流水账。

- 使用 Unreleased，按 Added、Changed、Fixed、Removed、Security 分类；无内容的分类无需保留。
- 记录实际变化及对使用者的影响；未来计划留在 SKILL.md，不写成已完成。
- 破坏性变化写出受影响范围和迁移办法；发布时归档到版本与日期。
- 修改 CHANGELOG 本身的整理可归入同一任务记录，不递归追加“更新日志的日志”。

仓库级 AGENT.md 是本项目约定的正文；[AGENTS.md](AGENTS.md) 提供供自动发现工具使用的入口，两者不能维护相互冲突的规则。
