# 插件 SDK 与协议指南

P2 已完成 Linux 本地验收：Rust SDK 封装握手、生命周期、有界通信、事件订阅、动作与版本化服务调用；echo 与 mock-adapter 是可运行的正式 SDK 示例。P1 独立协议 fixture 继续用于故障注入。P3 配置事务、本地包管理和排他热重载已完成 Linux 本地验收；P4 OneBot 类型库和 SnowLuma 正向 Universal WS 适配器已完成 Linux 本地验收及指定真实部署范围验证；reverse/split/HTTP 延期。P5 Web 管理已完成 Linux 本地验收，覆盖桌面/手机真实Chromium交互及CLI救援；Windows运行仍待验证。Cargo `0.1.0` 不代表稳定 SDK 已发布。

| 指南 | 内容 |
| --- | --- |
| [Web 管理与 CLI 救援](web-admin.md) | P5已完成Linux本地验收：登录、管理操作、诊断与Web不可用时恢复 |
| [适配器作者与 SnowLuma 运行指南](adapters.md) | 打包、配置、启用、WS状态与动作/事件边界 |
| [SnowLuma 兼容矩阵](../compatibility/snowluma.md) | 公开来源、mock及真实逐项证据 |
| [快速开始](quickstart.md) | 构建、打包、启动 echo/mock 与无账号闭环 |
| [事件](events.md) | 来源身份、订阅与投递缺口 |
| [动作](actions.md) | BotRef、结果未知和无自动重试 |
| [服务](services.md) | 显式提供者、SemVer 和退出清理 |
| [配置](configuration.md) | Schema、秘密引用、revision 与 desired/applied |
| [存储](storage.md) | 安装完整性、启用分离与数据保留 |
| [热重载](hot-reload.md) | 排他替换、排空、恢复与操作查询 |
| [迁移](migration.md) | 旧包、配置和业务数据升级步骤 |
| [协议](protocol.md) | 帧、信封、握手、请求与资源上限 |
| [清单与静态计划](manifest.md) | plugin.toml、host.toml、路径和依赖 |
| [生命周期](lifecycle.md) | 启停、失败、generation 与直接子进程清理 |
| [权限](permissions.md) | 申请与授予、默认拒绝、非沙箱边界 |
| [错误](errors.md) | wire 错误类别与恢复语义 |
| [排障与手工验证](troubleshooting.md) | CLI、fixture 打包、目录权限与超时 |

机器契约由 schemars 从 Rust 类型导出到 [schemas](../../schemas)：`protocol-v1.json`、`hello-v1.json`、`initialize-v1.json`、`plugin-manifest-v1.json`、`host-config-v1.json` 以及 P2 的 BotRef、注册/订阅/发布、事件、投递报告、动作与服务载荷 Schema，以及 P3 的 `config-snapshot-v1.json`、`config-prepare-v1.json`、`config-commit-v1.json`。Schema 描述结构；身份、路径、依赖、SemVer、权限和运行态还需宿主语义校验。重新生成时去掉下列 `-- --check`：

```sh
cargo run -p twintail-protocol --example export_schema -- --check
cargo run -p twintail-core --example export_manifest_schema -- --check
```

P3 配置、存储、热重载与迁移指南随实现核对，指南中的命令不构成已完成验证的声明。总体设计见 [SKILL.md](../../SKILL.md)，当前限制见[安全边界](../architecture/security.md)。
