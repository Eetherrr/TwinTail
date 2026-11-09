# 插件开发：从一条消息到自己的功能

TwinTail 插件是由宿主启动的独立程序。它可以接收消息事件、请求适配器执行动作，或向其他插件提供服务。Rust SDK 帮你处理进程通信和回调调度；你编写业务逻辑，不需要手动拼接协议帧，也不需要把 QQ 登录放进自己的插件。

## 先选一条适合你的路径

- **完全不了解项目，想自己写程序**：读[从零编写第一个插件](first-plugin.md)。它解释每段代码、源码路径依赖、清单、构建、权限和无账号验证。
- **只想先运行已有示例**：读[快速开始](quickstart.md)，先验证 mock→echo→动作记录，再修改自己的逻辑。
- **正在写代码，想查函数参数、返回或错误**：读[SDK 公共 API 参考](sdk-reference.md)。这是按实际 SDK 和 protocol 源码核对的接口清单。
- **想连接真实 QQ 或实现另一种协议**：读[适配器指南](adapters.md)；是否已验证你的部署方式，要查看[真实兼容矩阵](../compatibility/snowluma.md)，不是只看功能名。
- **只想管理已安装插件**：读[Web 管理与 CLI 救援](web-admin.md)，无需先学习 SDK。

## 先认识五个名字

| 名字 | 白话含义 | 进一步阅读 |
| --- | --- | --- |
| 宿主 host | 管理插件进程、权限、配置和路由的程序 | [生命周期](lifecycle.md) |
| 适配器 adapter | 连接外部协议服务、发布事件并执行动作的插件 | [适配器](adapters.md) |
| 事件 event | “收到了一条消息”这样的通知 | [事件](events.md) |
| 动作 action | “请发送一条消息”这样的请求 | [动作](actions.md) |
| BotRef | “哪个适配器实例上的哪个账号”，不是单独 QQ 号 | [SDK BotRef](sdk-reference.md) |

`Context` 是 SDK 交给回调的宿主客户端；`Hello` 是你的身份声明；`Plugin` 是你实现的回调接口；`run_stdio` 是实际运行这些回调的入口。它们的准确签名与区别都在[SDK 参考](sdk-reference.md)。不要把 Cargo 包版本、wire 协议版本、配置修订号和进程代次当作同一个版本号。

## 按问题查指南

| 问题 | 指南及其内容 |
| --- | --- |
| 怎样安排运行目录和依赖？ | [清单与启用计划](manifest.md)：plugin.toml、host.toml、路径限制与强依赖 |
| 为什么清单写了能力仍报错？ | [权限](permissions.md)：申请与授予的交集、精确能力名、信任边界 |
| 何时初始化和清理？ | [生命周期](lifecycle.md)：start、stop、取消、generation 和直接子进程 |
| 收不到事件或担心重复？ | [事件](events.md)：订阅、来源、准入计数、丢失与去重 |
| 怎样回复，超时能否重发？ | [动作](actions.md)：BotRef、参数与结果未知 |
| 怎样调用其他插件？ | [服务](services.md)：显式提供者、接口版本和回调 |
| 怎样读设置、在线修改或用秘密？ | [配置](configuration.md)：Schema、revision、快照、prepare/commit/abort、desired/applied |
| 文件写在哪里，卸载会删什么？ | [存储](storage.md)：包校验、数据目录、卸载和彻底移除 |
| 怎样升级而不停止整个宿主？ | [热重载](hot-reload.md)：排他替换、排空、恢复与操作查询 |
| 旧包、旧配置和数据库怎样升级？ | [迁移](migration.md)：备份、格式兼容与升级步骤 |
| 错误应该怎么处理？ | [错误](errors.md)：稳定 code 与恢复语义 |
| 如何测试包、排查失败？ | [契约工具](contract-tools.md)与[排障](troubleshooting.md) |
| 想写其他语言 SDK 或检查字节通信？ | [通信协议](protocol.md)：帧、信封、握手和资源上限 |

## 机器契约与源码核对

机器契约由 schemars 从 Rust 类型导出到 [schemas](../../schemas)：`protocol-v1.json`、`hello-v1.json`、`initialize-v1.json`、`plugin-manifest-v1.json`、`host-config-v1.json`，以及 BotRef、注册/订阅/发布、事件、投递报告、动作、服务、`config-snapshot-v1.json`、`config-prepare-v1.json` 和 `config-commit-v1.json`。Schema 说明结构；身份、路径、依赖、SemVer、权限和运行态还需宿主语义校验。

在仓库根检查生成物是否与源码一致；重新生成时去掉 `-- --check`：

```sh
cargo run -p twintail-protocol --example export_schema -- --check
cargo run -p twintail-core --example export_manifest_schema -- --check
```

## 当前边界与历史证据

SDK 封装握手、生命周期、有界通信、事件订阅、动作与版本化服务，支持配置事务、本地包管理和排他热重载。OneBot 类型与 SnowLuma 正向 Universal WS 已有指定真实部署验证；loopback 反向 Universal WS 有本地 mock 与子进程验证，真实 SnowLuma 反向部署尚未验证，split/HTTP 延期。Web 管理已有 Linux 本地验收；Windows 原生 CI 通过不等于 Windows live 浏览器专项通过。

SDK 的 Cargo `0.1.0` 不表示稳定 API 承诺，也不表示 main 的每一项改进已进入 v0.1.0 附件。独立进程不是操作系统沙箱，仅运行受信任插件；SDK 与宿主不会自动重试副作用动作，不承诺事件持久重放或零停机重载。

历史与证据放在这里查阅，而非作为学习 API 的先决条件：[总体设计](../../SKILL.md) · [架构决策](../architecture/README.md) · [平台矩阵](../release/platforms.md) · [真实 SnowLuma 兼容矩阵](../compatibility/snowluma.md) · [安全边界](../architecture/security.md)。指南中的示例不自动构成所有平台和外部服务均已验证的声明。
