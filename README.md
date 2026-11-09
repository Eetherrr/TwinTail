# TwinTail

**用 Rust 写 QQ 机器人，把接入、功能和管理都交给插件。**

[文档网站](https://eetherrr.github.io/TwinTail/) · [快速开始](docs/plugin-api/quickstart.md) · [编写插件](docs/plugin-api/README.md) · [下载](https://github.com/Eetherrr/TwinTail/releases) · [MIT](LICENSE)

TwinTail 帮你启动和管理机器人的插件。你可以用它接收消息、调用发送消息等操作，也可以自己写插件，添加回复、查询或其他业务功能。插件出问题时，框架会记录状态；你可以通过命令行或网页查看和处理。

**它不是 QQ 登录客户端。** QQ 登录和账号连接由独立运行的 SnowLuma 负责，TwinTail 通过 `adapter-snowluma` 插件连接它。两者用 OneBot v11 这套通信规则交换消息。没有 QQ 账号也能先跑仓库里的模拟示例。

## 能做什么？

- **管理插件**：安装多个版本，选择启用哪个版本，启动、停止和重载插件。
- **修改配置**：先检查配置格式，再保存和交给正在运行的插件；页面能看到配置是否已经生效。
- **用网页操作**：查看插件状态、阅读插件说明、编辑配置、跟踪操作结果。网页管理本身也是插件，不需要时可以不开。
- **开发自己的功能**：Rust SDK 帮你处理与框架的通信；你主要编写收到消息后做什么、何时调用其他功能。
- **保留恢复手段**：即使网页管理停止，仍可用本机命令行恢复。普通卸载保留数据，另有明确确认的“彻底移除”操作。

插件各自作为独立程序运行，不是把动态库加载进宿主。请只运行可信插件：这里的权限检查约束框架接口，并不把插件与电脑文件隔离。

## 第一次用，从哪里开始？

| 你的目标 | 推荐入口 |
| --- | --- |
| 先看看框架如何工作，不连接 QQ | [运行模拟消息示例](docs/plugin-api/quickstart.md) |
| 连接自己运行的 SnowLuma | [接入 QQ 的步骤](docs/plugin-api/adapters.md) |
| 打开网页管理插件 | [网页管理与命令行恢复](docs/plugin-api/web-admin.md) |
| 用 Rust 写第一个插件 | [插件开发入门](docs/plugin-api/README.md) |
| 升级、备份、发布新版本 | [运维与发布](docs/release/README.md) |

### 只想先确认能编译？

准备好 Git、rustup 和系统所需的编译/链接工具。克隆仓库后运行：

```sh
git clone https://github.com/Eetherrr/TwinTail.git
cd TwinTail
cargo run -p twintail-host --bin twintail-host --locked -- --check
```

仓库会通过 `rust-toolchain.toml` 选择 Rust **1.99.1**；第一次运行可能需要下载工具链和依赖。`--check` 成功后直接退出，不连接 QQ，也不创建运行目录。

要启动宿主：

```sh
cargo run -p twintail-host --bin twintail-host --locked
```

它会读取数据目录里的 `host.toml`，启动里面明确配置的插件，按 Ctrl-C 退出。**空目录启动不会自动安装插件，也不会自动出现网页或机器人**。继续按[快速开始](docs/plugin-api/quickstart.md)组装并运行示例。

`--validate` 用来读取并检查配置与插件包，但不执行插件；`--help` 查看完整命令。默认安装目录是 `packages/`，默认数据目录是 `data/`，可用 `--install-dir` 和 `--data-dir` 更改。Unix 上数据目录须由当前用户拥有且权限为 `0700`，框架不会悄悄修改已有目录权限。

## 插件装在哪里？卸载会删什么？

```text
packages/                     # 插件程序：同一个插件可以保留多个版本
  插件ID/
    版本/
      plugin.toml             # 名称、入口、权限等信息
      README.md               # 插件作者提供的使用说明
      插件可执行文件
      checksums.json          # 文件摘要，用来检查内容是否变化
data/                         # 配置和运行数据，不放在程序目录里
  plugins/插件ID/
    config.toml
    data/
```

- **卸载某个版本**：只删除对应版本的程序，保留配置和业务数据，方便重装。
- **彻底移除插件**：先停用，再输入插件 ID 确认，删除所有已安装版本及宿主管理的配置和业务数据。删除前请备份。
- 插件自行创建的外部文件、数据库和环境变量不会被一并删除；失败时应查看操作阶段，修复原因后重试。

网页里的“插件说明”读取包内 README，以纯文本显示。没有说明文件的旧插件包也可以使用。更多细节见[插件存储与清理](docs/plugin-api/storage.md)。

## 当前版本与支持范围

项目仍在早期，接口可能继续调整。**文档网站跟随 main 分支，不一定与旧版下载附件完全相同。**

- 当前工程版本是 `0.1.1`，包含插件说明阅读、彻底移除及新版入门与 SDK 文档，见[变更记录](CHANGELOG.md)。
- 已公开的 v0.1.0 目前提供 Linux x86_64 附件。新 Release 流程会在三个平台构建并生成草稿，是否有对应附件请以[下载页](https://github.com/Eetherrr/TwinTail/releases)为准。
- Linux x86_64、Linux arm64 和 Windows x86_64 MSVC 已有自动测试记录，具体见[平台说明](docs/release/platforms.md)。自动测试不等于每个平台都完成了真实 QQ 部署。
- 已在一套指定的 SnowLuma 部署上验证正向连接、私聊发送和断线重连。反向连接目前主要由模拟测试覆盖，其他传输方式尚未提供；详见[SnowLuma 兼容记录](docs/compatibility/snowluma.md)。

## 参与开发

先阅读[开发约定](AGENT.md)和[整体设计](SKILL.md)。接口与文档一起维护，每项改动记入[变更记录](CHANGELOG.md)。常用检查：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
python3 -m unittest discover -s tools/release -p 'test_*.py'
```

Rust 文档检查还应设置 `RUSTDOCFLAGS=-D warnings`。修改网页管理前端时，按 [WebUI 指南](docs/plugin-api/web-admin.md)使用固定 Node/npm 版本重新测试并打包资源。修改文档站时安装 `requirements-docs.txt`，再运行 `python3 tools/docs/check.py`。

想了解为什么这样设计，可读[架构说明](docs/architecture/README.md)；遇到问题先看[排障指南](docs/plugin-api/troubleshooting.md)。

## 许可证

项目代码采用 [MIT](LICENSE)。第三方组件保留各自许可证，见[第三方声明](THIRD_PARTY_NOTICES.md)。
