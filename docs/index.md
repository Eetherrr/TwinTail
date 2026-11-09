---
title: TwinTail · 文档
hide:
  - toc
---

<div class="tt-hero" markdown>

<span class="tt-eyebrow">RUST · ONEBOT V11 · PLUGINS</span>

# 用插件搭建你的 QQ 机器人

TwinTail 帮你运行和管理机器人插件：接收消息、交给插件处理，再让插件发送回复。你可以先用不连接 QQ 的示例试一遍，随后接入自己部署的 SnowLuma，并在本机浏览器中管理插件。

[第一次运行：不需要 QQ 账号](plugin-api/quickstart.md){ .md-button .md-button--primary }
[用浏览器管理插件](plugin-api/web-admin.md){ .md-button }

</div>

## 先分清这几个东西

| 名称 | 大白话解释 | 由谁运行 |
| --- | --- | --- |
| **TwinTail 宿主** | 插件的“管理员”：启动、停止插件，转发消息，检查权限和配置 | 你在电脑或服务器上启动 `twintail-host` |
| **插件** | 做具体事情的小程序，例如回复消息、连接 QQ 服务或提供管理网页 | 由宿主启动为独立进程 |
| **OneBot v11** | 机器人程序之间约定的消息格式和通信规则，不是一个需要登录的 QQ 客户端 | TwinTail 适配器与外部服务都按这套规则通信 |
| **SnowLuma** | 提供 QQ 接入与 OneBot 服务的独立程序 | **需要你另外部署和登录**，不随 TwinTail 自动安装 |
| **适配器** | “翻译插件”：把 SnowLuma 的 OneBot 消息转成 TwinTail 插件可接收的事件，并把回复动作送回去 | `adapter-snowluma` 插件 |
| **TwinTail WebUI** | 管理你这台机器上 TwinTail 插件的网页 | `web-admin` 插件提供，默认 `http://127.0.0.1:17280/` |
| **本文档网站** | 你正在阅读的说明书，只展示文档 | 静态网站；没有机器人管理、登录或配置修改功能 |

SnowLuma 自己的管理网页负责 QQ 登录等账号运维；TwinTail WebUI 负责 TwinTail 的插件与配置。它们和本文档网站是三个不同入口。

## 你现在可以做什么

<div class="grid cards" markdown>

-   :material-rocket-launch-outline: **先跑通，不用 QQ 账号**

    ---

    从源码构建宿主和两个示例，手动注入一条假消息，看到回复动作记录。适合第一次接触机器人框架的人。

    [一步一步运行示例 →](plugin-api/quickstart.md)

-   :material-connection: **连接自己的 SnowLuma**

    ---

    在 SnowLuma 已独立部署后，设置地址、账号和凭据，安装适配器，检查连接是否成功。

    [接入与连接排查 →](plugin-api/adapters.md)

-   :material-monitor-dashboard: **在浏览器里管理**

    ---

    安装本机管理插件，登录后查看插件、启停、安装本地包和修改配置。网页故障时仍可用命令行恢复。

    [安装 WebUI 与日常操作 →](plugin-api/web-admin.md)

-   :material-puzzle-outline: **编写自己的功能插件**

    ---

    用 Rust SDK 接收事件、发起动作或提供服务。插件需要明确授权；独立进程不等于系统沙箱，请只运行受信任插件。

    [插件开发指南 →](plugin-api/README.md)

</div>

## 该下载成品还是从源码构建？

- **想直接使用现成程序**：先看 [GitHub Releases](https://github.com/Eetherrr/TwinTail/releases)，按[部署说明](release/operations.md)核对系统、CPU 架构与摘要。v0.1.0 已发布的附件只有 Linux x86_64。
- **想跟随最新 main、修改程序或运行 Cargo 示例**：取得完整源码，按[快速开始](plugin-api/quickstart.md)安装固定工具链再构建。预编译包不含完整源码，解包后直接运行 Cargo 示例不会具备所需工程。
- **想发布新版本**：阅读[发行入口](release/README.md)和[自动化发行](release/automation.md)。新自动化尚未取得完整发行运行通过的记录；手动入口仅构建，tag 流程只创建草稿，公开发布仍由人工完成。

## 怎么知道真的成功了？

不要只看“程序还在运行”。按顺序确认：宿主能查询插件 → 插件已启动 → 配置已应用 → 适配器已连接 → 实际业务得到预期结果。每篇操作指南都给出成功标志和出错时的检查方法。

这里的“事件”是一条收到的消息或通知，“动作”是插件要求机器人执行的事情，例如发消息。本地 mock 示例中的动作记录证明 TwinTail 内部链路可用，并不表示向 QQ 发出了消息。

!!! note "阅读与你运行的版本相符的说明"
    本网站跟随 main。当前 Cargo 工程版本为 `0.1.0`，插件通信协议 wire 为 `1.2`，这是两套不同版本号。main 的新功能以[变更历史](../CHANGELOG.md)中 Unreleased 为准，不自动出现在 v0.1.0 附件里。[平台矩阵](release/platforms.md)有 Linux x64/arm64、Windows MSVC 和 WebUI 的原生 CI 记录；真实 QQ、反向连接等范围仍需逐项看[SnowLuma 兼容矩阵](compatibility/snowluma.md)。

## 继续查阅

| 你遇到的问题 | 对应说明 |
| --- | --- |
| 升级、备份或恢复数据 | [用户运维](release/operations.md) |
| 插件怎么打包交给别人 | [构建与交付](release/plugins.md) |
| 哪些系统实际测过 | [平台验证矩阵](release/platforms.md) |
| 为什么这样设计 | [架构决策](architecture/README.md) · [安全边界](architecture/security.md) · [总体设计](../SKILL.md) |
| 想在本地预览本文档网站 | [网站预览与部署](release/project-review.md) |

网站直接使用仓库 Markdown，不维护另一份说明书。源文件、Schema 和仓库级文档链接会打开 GitHub 上的对应位置。

[GitHub 仓库](https://github.com/Eetherrr/TwinTail) · [变更历史](../CHANGELOG.md) · [发行记录](https://github.com/Eetherrr/TwinTail/releases)
