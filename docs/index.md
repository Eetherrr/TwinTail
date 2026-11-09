---
title: TwinTail · 文档
hide:
  - toc
---

<div class="tt-hero" markdown>

<span class="tt-eyebrow">RUST · ONEBOT V11 · PLUGINS</span>

# 轻盈宿主，无限延展。

TwinTail 是以 **Rust** 为核心的 QQ Bot 框架。用独立进程承载插件，用版本化协议连接能力，让接入、业务与管理各归其位。

[开始探索](plugin-api/quickstart.md){ .md-button .md-button--primary }
[浏览插件 API](plugin-api/README.md){ .md-button }

</div>

<div class="grid cards" markdown>

-   :material-rocket-launch-outline: **从一个可运行的闭环开始**

    ---

    无需 QQ 账号，用 echo/mock 示例理解事件与动作，再接入自己的 OneBot 服务。

    [快速开始 →](plugin-api/quickstart.md)

-   :material-puzzle-outline: **一切皆插件，边界清晰**

    ---

    独立进程、版本化 IPC、显式权限与有界队列。SDK 提供便利封装，不依赖跨进程 Rust ABI。

    [开发者指南 →](plugin-api/README.md)

-   :material-monitor-dashboard: **管理可视，恢复有路**

    ---

    WebUI 与 CLI 共用宿主管理服务；查看配置修订、操作状态，并保留本机救援入口。

    [管理与运维 →](plugin-api/web-admin.md)

-   :material-shield-check-outline: **让证据说明兼容性**

    ---

    实现范围、mock、本地验收与真实 SnowLuma 记录分别维护，不用单项测试代替全部平台结论。

    [验证矩阵 →](compatibility/README.md)

</div>

!!! note "当前文档跟随 main，而不是旧发行附件"
    当前工程版本为 `0.1.0`，IPC wire 为 `1.2`，两者不是同一版本体系。v0.1.0 已公开发行，但 main 新增的插件 README 与彻底移除等功能以 Unreleased 为准。Linux x64/arm64、Windows MSVC 和 WebUI 已有[原生 CI 证据](release/platforms.md)；这不扩大真实 QQ 或反向 SnowLuma 验证范围。

## 选择你的路径

| 你想做什么 | 从这里进入 |
| --- | --- |
| 部署与使用 | [快速开始](plugin-api/quickstart.md) · [SnowLuma 接入](plugin-api/adapters.md) · [冷备与恢复](release/operations.md) |
| 编写插件 | [清单](plugin-api/manifest.md) · [生命周期](plugin-api/lifecycle.md) · [事件](plugin-api/events.md) · [动作](plugin-api/actions.md) |
| 理解设计 | [架构决策](architecture/README.md) · [安全边界](architecture/security.md) · [项目总体设计](../SKILL.md) |
| 准备发行 | [发行入口](release/README.md) · [插件发布](release/plugins.md) · [平台矩阵](release/platforms.md) |

!!! info "实现与验证，分开阅读"
    TwinTail 已提供独立插件宿主、SDK、配置事务、排他热重载、SnowLuma Universal WS 接入与 Web 管理。真实 SnowLuma 证据仍以[专项兼容矩阵](compatibility/snowluma.md)为准；反向 WS、其他传输及平台的实际验证范围请逐项核对。

## 同一份文档，两个入口

本网站直接构建仓库中的现有 Markdown，开发指南、ADR 与兼容记录不维护网站专用副本。站内页面可从右上角查看源文件或提出修改；源码、Schema 和仓库级文档链接会打开 GitHub 上的对应位置。

[GitHub 仓库](https://github.com/Eetherrr/TwinTail) · [变更历史](../CHANGELOG.md) · [发行记录](https://github.com/Eetherrr/TwinTail/releases)
