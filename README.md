# TwinTail

[GitHub](https://github.com/Eetherrr/TwinTail) · [Releases](https://github.com/Eetherrr/TwinTail/releases) · [MIT](LICENSE)

以 Rust 为核心、采用 OneBot v11 的 QQ Bot 框架。目标是以插件提供协议接入、业务功能与 WebUI；首个适配器通过 Universal WebSocket 连接独立部署的 SnowLuma。

P0–P5 已完成 Linux 本地验收：包含 Rust SDK、独立进程插件管理与配置、排他热重载、事件/动作/服务路由、SnowLuma 适配器及 Vue WebUI。无 QQ 账号可按 [SDK quickstart](docs/plugin-api/quickstart.md) 运行 echo/mock 闭环。

P6 增加反向 Universal WebSocket、候选打包与校验、冷备份恢复、插件契约和有界负载工具。Linux 候选及实际验证范围见[发行指南](docs/release/README.md)与[兼容性记录](docs/compatibility/README.md)。指定真实 SnowLuma 部署的正向私聊/重连已验证；反向接入目前只有本地 mock 证据。split/HTTP 传输仍延期。Linux x64、Linux arm64 与Windows x64 MSVC已通过本次GitHub native CI，运行范围见[平台矩阵](docs/release/platforms.md)；发行附件目前仅Linux x64。项目采用 [MIT 许可证](LICENSE)，源码main已公开；v0.1.0 Release状态以[发行页面](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0)为准，候选工作流不会自动发布。架构和阶段门槛见 [SKILL.md](SKILL.md)。

## 开始运行

需要 rustup、目标平台的 Rust 链接工具链和网络访问依赖源。仓库固定 Rust **1.99.1**（同时为当前 MSRV），rustup 会读取 `rust-toolchain.toml`；首次使用会下载该版本及 rustfmt、Clippy。

```sh
cargo run -p twintail-host --bin twintail-host --locked -- --check
cargo run -p twintail-host --bin twintail-host --locked
```

本机固定 `1.99.1` 工具链已安装并用于 P1 验收。首次安装遇到下载等待时，如果已安装同版本 `stable`，可确认 `rustc +stable --version` 为 `1.99.1`，再临时使用 `cargo +stable run -p twintail-host --bin twintail-host --locked -- --check`；CI 与项目配置仍固定 `1.99.1`。

第一条命令只检查引导后退出，不读取配置或创建目录；第二条读取数据根内存在的 `host.toml`，校验并启动显式配置的插件，然后等待 Ctrl-C，Unix 同时支持 SIGTERM。配置不存在表示空计划。离线读取配置和包但不执行插件可用 `--validate`。帮助和版本：

```sh
cargo run -p twintail-host --bin twintail-host --locked -- --help
cargo run -p twintail-host --bin twintail-host --locked -- --version
```

可使用 `--data-dir PATH` 与 `--install-dir PATH` 指定目录。默认分别是当前工作目录下的 `data/` 与 `packages/`。正常运行会创建/检查私有数据根，并绑定本机管理端点；Unix 数据根要求当前有效用户拥有且权限精确为 0700，端点为 `data/management.sock`（0600）。已有不安全目录不会自动修改，残留 socket 需确认宿主已停止后手动清理。路径含空格时请加引号；以 `-` 开头的相对路径需加 `./` 前缀。P0 历史规则见 [ADR 0001](docs/architecture/0001-p0-baseline.md)，P1 行为见 [插件 API 指南](docs/plugin-api/README.md)。

基础管理（宿主已运行时，在另一终端执行）：

```sh
cargo run -p twintail-host --bin twintail-host -- plugins list --data-dir ./data
cargo run -p twintail-host --bin twintail-host -- plugins status ID --data-dir ./data
cargo run -p twintail-host --bin twintail-host -- plugins start ID --data-dir ./data
cargo run -p twintail-host --bin twintail-host -- plugins stop ID --data-dir ./data
cargo run -p twintail-host --bin twintail-host -- plugins call ID METHOD '{}' --data-dir ./data
```

SDK 示例与 mock 闭环见 [quickstart](docs/plugin-api/quickstart.md)。清单、权限、wire 协议与独立协议 fixture 手工打包流程见 [插件 API 文档](docs/plugin-api/README.md)。P1 fixture 用于协议故障注入；仅运行受信任本地插件，框架权限不构成沙箱。

SnowLuma 打包、首次配置、启用和连接诊断见 [适配器快速指南](docs/plugin-api/adapters.md)，真实版本与逐项证据见 [SnowLuma 兼容矩阵](docs/compatibility/snowluma.md)。echo 连接真实适配器使用 SnowLuma 专用清单；mock 快速开始保留独立用途。

P5 Web 管理运行：从干净目录打包、随机管理凭据、首次启动、登录与CLI救援见 [Web管理指南](docs/plugin-api/web-admin.md)。默认入口 `http://127.0.0.1:17280/`，只允许loopback。Rust可直接构建已嵌入资源；修改前端时在web目录用固定Node26.7.0/npm12.2.1执行 `npm ci`、`npm run typecheck`、`npm test`、`npm run build`，再运行 `cargo build --locked -p web-admin`；浏览器检查用 `npm run test:browser`，OpenAPI漂移用 `cargo run --locked -p web-admin --example export_openapi -- --check`。最终验收另见兼容性记录。

## 本地检查

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

文档检查应将 `RUSTDOCFLAGS` 设为 `-D warnings`（POSIX shell：`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`；PowerShell：先运行 `$env:RUSTDOCFLAGS="-D warnings"`）。CI 使用固定工具链在 Linux 和 Windows 运行上述检查；远端结果以实际 Actions 记录为准。

## 项目入口

- [AGENT.md](AGENT.md)：开发执行约定，所有项目改动同步记录 CHANGELOG。
- [SKILL.md](SKILL.md)：整体架构、插件契约和 P0–P6 计划。
- [架构与 ADR](docs/architecture/README.md)：已采纳的实现决策。
- [安全边界](docs/architecture/security.md)：当前能力与后续防护目标。
- [兼容性记录](docs/compatibility/README.md)：工具链、平台与 SnowLuma 验证状态。
- [CHANGELOG.md](CHANGELOG.md)：项目变更历史。

P3 的实际能力和限制见 [插件 API 文档](docs/plugin-api/README.md)与 [ADR 0004](docs/architecture/0004-p3-management-config-reload.md)。

## 许可证

项目代码采用 [MIT](LICENSE) 许可证。第三方依赖保留各自许可，随包声明和许可正文见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
