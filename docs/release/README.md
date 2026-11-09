# 下载、部署与发行

想把 TwinTail 跑起来，先选适合自己的程序来源；想给别人提供新版本，再看构建和发行说明。TwinTail 使用 MIT 许可证，源码位于公开仓库 main 分支，[v0.1.0 Release](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0) 已发布 **Linux x86_64** 附件。main 的新功能不自动进入旧附件，使用时请核对版本。

## 我只是想用，应该从哪里开始？

1. **下载现成程序**：到 Release 页面选择与你的系统和 CPU 架构相符的附件；只有源代码归档而没有对应二进制时，不要把其他平台的包当作可直接运行的程序。v0.1.0 已公开的二进制仅 Linux x86_64。
2. **核对下载**：比较页面提供的 SHA256 与下载文件，再按[运维指南](operations.md)解包、设置独立数据根。SHA256 是文件“指纹”，用于检查下载损坏或内容变化，不是发行者签名。
3. **先跑一个示例**：[快速开始](../plugin-api/quickstart.md)从源码运行不需要 QQ 的 mock/echo 示例。
4. **接入真正机器人**：自行部署和登录 SnowLuma，再按[适配器指南](../plugin-api/adapters.md)连接它的 OneBot 服务。需要网页管理时另装 [web-admin](../plugin-api/web-admin.md)。

预编译包包含宿主、插件程序、白名单文档与工具，不是完整源码仓库。使用 `cargo build` 或 Cargo 示例（插件合同测试、负载测试等）需要取得**同版本完整源码**；不要在解包目录期待缺失的源码自行出现。Python 状态工具只依赖标准库，但仍受 Unix/停机限制。

## 这些发行名词是什么意思？

| 名称 | 含义 |
| --- | --- |
| 版本 tag | 指向某个源码提交的版本标签，例如 `v0.1.0`；不是运行配置 |
| 归档 / 制品 | 构建出的程序和文件，通常压成 `.tar.gz` 或 `.zip` 供下载 |
| 本地待发布包（工具标记为 candidate） | 尚待复查的构建结果；能打包不等于已发布或业务可用 |
| Actions artifact | 工作流临时保存的构建结果，不等于公开 Release 附件 |
| Release 草稿 | GitHub 上尚未公开的发行记录，需要人工审核后发布 |
| 平台验证 | 在指定系统/架构上实际执行了哪些检查；编译、启动和真实 QQ 业务是不同层次 |

## 按任务找说明

| 你要做什么 | 阅读 |
| --- | --- |
| 安装、日常检查、升级和数据备份 | [用户运维](operations.md) |
| 从源码生成包，给插件做测试 | [构建与交付](plugins.md) · [插件合同工具](../plugin-api/contract-tools.md) |
| 用 GitHub Actions 构建新版本并准备草稿 | [自动化发行](automation.md) |
| 核对某平台究竟测过什么 | [平台矩阵](platforms.md) · [真实 SnowLuma 范围](../compatibility/snowluma.md) |
| 审查依赖、凭据和许可 | [安全与依赖升级](security-updates.md) · [历史依赖审查](dependency-audit.md) |
| 本地预览或部署文档网站 | [网站维护与审查记录](project-review.md) |

## 发布者需要保留的边界

发行工具入口是 `python3 tools/release/release.py --help`，构建、整理文件、压缩和校验步骤见[构建与交付](plugins.md)。工具报告固定保留 `channel=candidate`、`public_release=false`、`runtime_verified=false`：这些字段描述打包工具自身的证据，不会因为另外公开了 Release 就自动改写为业务验证通过。

Release 自动化的 v0.1.1 首次运行已通过完整 CI、三平台原生打包、汇总和草稿创建；首次公开验证失败后保留草稿，恢复详情见[自动化发行](automation.md)。手动入口仅构建并上传 review artifacts；普通 tag 创建草稿，明确批准的附注 tag 才会在全部核验成功后公开。它不自动发布 Cargo registry，也不修改现有 v0.1.0。

再分发制品应包含项目 MIT 许可与需要的第三方声明，并重新执行 stage/verify。Linux/Windows 原生 CI、浏览器测试和真实 QQ 连接分别记录，任何一项都不扩大其他项的结论；反向真实 SnowLuma 尚未验证。
