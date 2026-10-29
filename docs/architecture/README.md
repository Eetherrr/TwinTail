# TwinTail 架构文档

当前状态：**P3 已完成 Linux 本地验收；P4 OneBot/SnowLuma 已完成 Linux 本地验收及指定真实部署范围验证**。总体设计与后续阶段见 [SKILL.md](../../SKILL.md)，执行约定见 [AGENT.md](../../AGENT.md)，变更历史见 [CHANGELOG](../../CHANGELOG.md)。

Rust 2024 / resolver 3 workspace 核心 crate：`twintail-protocol` 定义 wire 类型与有界帧 IO；`twintail-core` 实现清单、精确权限交集、依赖加载计划与进程监管；`twintail-host` 组合 CLI、信号退出和本机管理。宿主不依赖某个插件实现。十七份 JSON Schema 由 schemars 导出并可检查漂移。

P1 正常运行读取 DATA/host.toml，缺失时为空计划；先绑定管理端点再启动插件，退出时逆序停止。`--check` 保持引导检查、不读写；`--validate` 离线检查包、不执行。基础管理为 plugins list/status/start/stop/call。Unix 本机管理使用私有目录和 peer UID 校验，Windows 等价后端已实现并通过交叉编译/Clippy，运行未验证。

P2 新增 `twintail-plugin-sdk`、BotRef、有界事件/动作/服务路由，以及 echo/mock 示例。P3 已实现配置事务、安装器与排他热重载。P4 新增 twintail-onebot11 和独立 adapter-snowluma，支持正向 Universal WS；reverse/split/HTTP 延期。P5 新增独立 web-admin、Vue 管理台与有界授权桥接；仅 loopback HTTP、JSON 配置编辑与安全操作诊断，见 [ADR 0006](0006-p5-web-admin.md)。独立进程不构成沙箱；监管只覆盖直接子进程，禁止派生守护子进程。详细契约见[插件 API](../plugin-api/README.md)。

| 文档 | 内容 |
| --- | --- |
| [ADR 0001：P0 基线](0001-p0-baseline.md) | 历史工程与空宿主决策，保留 P0 当时的路径语义 |
| [ADR 0002：P1 插件宿主](0002-p1-plugin-host.md) | P1 协议、包计划、监管和本机管理决策 |
| [ADR 0003：P2 SDK 与路由](0003-p2-sdk-routing.md) | 路由身份、背压、版本服务和 SDK 并发契约 |
| [ADR 0004：P3 管理与重载](0004-p3-management-config-reload.md) | 包管理、配置事务、排他替换及恢复 |
| [ADR 0005：P4 OneBot 与 SnowLuma](0005-p4-onebot-snowluma.md) | 正向 Universal WS、无损类型、有界连接和未知结果 |
| [ADR 0006：P5 Web 管理](0006-p5-web-admin.md) | 独立HTTP插件、授权桥接、会话与CLI恢复 |
| [ADR 0007：P6 发布加固](0007-p6-release-hardening.md) | 反向 Universal WS、候选打包、冷备份与负载验证（实施中） |
| [适配器指南](../plugin-api/adapters.md) | 构建、打包、配置、启用与连接诊断 |
| [插件 API](../plugin-api/README.md) | 当前实际 wire、清单、权限、生命周期和排障 |
| [安全边界](security.md) | 当前已有控制与未覆盖的信任边界 |
| [兼容性与验证矩阵](../compatibility/README.md) | 平台、阶段及 SnowLuma 证据状态 |

协议、权限、依赖方向、热重载或持久化兼容的重大变更须同步 ADR 与设计。验收结论需具体环境和证据，不能把存在实现或通过 mock 写成已验收/真实外部兼容。
