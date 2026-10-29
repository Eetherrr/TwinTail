# Rust 插件 quickstart

P2 SDK、路由与示例已完成 Linux 本地验收。下面从仓库源码构建，在全新的目录内组装只含清单和可执行文件的包，并通过宿主本机管理接口演示 mock 事件 → echo → mock 动作。这里不需要 QQ 账号；配置事务和热重载属于 P3，真实 QQ/SnowLuma 属于 P4。

2026-10-29 Linux 本地已按本页包结构完成独立 release 构建、两包离线校验、CLI 注入与查询、顺序停用：mock.emit 返回 delivered=1/dropped=0，mock.actions 记录一条携带相同消息段的 send_msg，宿主正常退出。此项验证仅覆盖本页演示，完整 P2 检查见[兼容性记录](../compatibility/README.md)；Windows 运行仍待验证。

使用仓库固定的 Rust 1.99.1 工具链与 Cargo.lock。SDK、protocol、core、host 和两个示例的 Cargo 包版本目前均为 0.1.0；它们不是 wire 版本。wire 为 1.2，清单 `protocol_version = 1` 表示主版本，`host_api = "^0.1.0"` 表示宿主 API 兼容范围。当前 SDK 接受 wire 1.1 或兼容的更高 minor，低于 1.1 会拒绝初始化；配置及应用启动确认扩展要求双方支持 1.2。SDK 通过版本化 JSON IPC 工作，不依赖 Rust 跨进程 ABI。必须授予的 P2 能力也参与握手校验，不能仅凭 wire major 相同推断旧宿主支持路由。

以下是 Linux/macOS 风格 shell 流程。当前平台验收结果以[兼容性记录](../compatibility/README.md)为准，命令示例不代表所有平台均已运行验证。在仓库根执行，保留当前终端供后续命令使用：

```sh
REPO="$PWD"
DEMO="$(mktemp -d "${TMPDIR:-/tmp}/twintail-p2.XXXXXX")"
export DEMO
cargo build --locked --release --target-dir "$DEMO/build" \
  -p twintail-host -p echo-plugin -p mock-adapter
mkdir -p "$DEMO/bin" "$DEMO/packages/echo-plugin" \
  "$DEMO/packages/mock-adapter" "$DEMO/data"
chmod 700 "$DEMO/data"
cp "$DEMO/build/release/twintail-host" "$DEMO/bin/"
cp "$DEMO/build/release/echo-plugin" "$DEMO/packages/echo-plugin/"
cp "$DEMO/build/release/mock-adapter" "$DEMO/packages/mock-adapter/"
cp "$REPO/examples/echo-plugin/plugin.toml" "$DEMO/packages/echo-plugin/"
cp "$REPO/examples/mock-adapter/plugin.toml" "$DEMO/packages/mock-adapter/"
cat > "$DEMO/data/host.toml" <<'TOML'
[[plugins]]
path = "mock-adapter"
grants = ["bots.register", "events.publish", "services.provide"]

[[plugins]]
path = "echo-plugin"
grants = ["events.subscribe", "actions.call"]
TOML
"$DEMO/bin/twintail-host" --validate \
  --data-dir "$DEMO/data" --install-dir "$DEMO/packages"
printf 'Demo directory: %s\n' "$DEMO"
```

这会在新目录构建全部三份二进制；运行包不依赖仓库 target 目录或 Cargo 源码。不要直接把 examples 目录作为安装包：清单的 entrypoint 指向包内二进制，Cargo.toml 和 src 本身不能被宿主执行。echo 的清单强依赖 mock-adapter；宿主按拓扑启动、逆序停止。离线 `--validate` 不启动插件。

启动已打包宿主并保持前台运行：

```sh
"$DEMO/bin/twintail-host" \
  --data-dir "$DEMO/data" --install-dir "$DEMO/packages"
```

在第二个终端将 DEMO 设置为上一步打印的实际绝对路径，然后运行：

```sh
DEMO=/tmp/twintail-p2.替换为实际目录
"$DEMO/bin/twintail-host" plugins list --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.ready '{}' \
  --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call echo-plugin echo.ready '{}' \
  --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.emit \
  '{"message_type":"private","user_id":"10002","message":[{"type":"text","data":{"text":"hello TwinTail"}}],"extension":{"preserved":true}}' \
  --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.actions '{}' \
  --data-dir "$DEMO/data"
```

注入前确认 mock.ready 与 echo.ready 均返回 `value.ready = true`；宿主 list 中的 active 不等于插件 start 回调已经完成。这两个探针确认机器人/服务注册与 echo 订阅初始化完成。

`plugins call ID METHOD JSON` 的 JSON 必须是一个 shell 参数，返回值打印为 `{"status":"ok","value":...}` 或 err outcome。mock.emit 将这个 JSON 作为 message 事件的原始 payload 发布，返回 delivered/dropped 准入计数。echo 订阅 message，保留 message、message_type、user_id、group_id，向 event.bot 调用 send_msg；它不把任意扩展字段改写为动作参数。mock.actions 返回记录的 ActionCall 数组，其中应出现 `action = "send_msg"` 和相同文本消息段。

publish 返回成功时 echo 可能尚未处理，首次 mock.actions 为空可稍后再次查询；不要为等待记录而重复 mock.emit，否则会产生第二条动作。delivered 代表队列准入，参阅[事件指南](events.md)。mock 记录证明本地路由闭环，不证明任何外部消息发送。

CLI 以当前 OS 用户经独立本机管理通道调用插件自定义 request；这与普通插件的 `services.call` 权限不同。`mock.emit` 和 `mock.actions` 是示例调试入口，不是宿主通用方法。mock 同时提供 `mock.inspect` 1.0.0 版本化服务，原样返回调用 params；插件调用方式见[服务指南](services.md)。

停止时先停止依赖者，再停止 mock，或在宿主终端按 Ctrl-C 关闭所有实例：

```sh
"$DEMO/bin/twintail-host" plugins stop echo-plugin --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins stop mock-adapter --data-dir "$DEMO/data"
```

停止被活动 echo 依赖的 mock 会得到 conflict。P2 没有通用 install、配置更新事务或 reload 命令；重新启用包可使用 plugins start，重新启动后 BotRef 属于新 generation，不复用旧实例路由。

Windows 构建产物包含 `.exe`。在 PowerShell 的新目录构建与复制时，对三个 bin 使用 `twintail-host.exe`、`echo-plugin.exe`、`mock-adapter.exe`；两个包内 plugin.toml 的 entrypoint 也分别改为 `echo-plugin.exe`、`mock-adapter.exe`。插件 id、包目录名与 capabilities 保持示例原值。使用 `New-Item -ItemType Directory` 创建目录、`Copy-Item` 复制文件，以 `& "$demo\bin\twintail-host.exe" ...` 调用。设置与 Unix 示例等价的 host.toml 后，先 --validate，再前台启动；管理使用宿主实现的本机命名管道，不创建 management.sock。JSON 仍必须作为单参数；PowerShell 可用 `'{"message":[{"type":"text","data":{"text":"hello"}}]}'`，并按所用 PowerShell 版本的原生命令参数规则传递。Windows 运行验收仍须目标环境完成。

编写自己的插件时实现 SDK 的 `Plugin` trait，并调用 `run_stdio(plugin, Hello { ... })`。Hello 的 plugin_id、version、required_capabilities 须与清单一致；`Plugin::start` 在宿主 activate 后注册机器人/服务或订阅，`event`、`action`、`service` 分别处理三类业务。Context 提供宿主初始化身份、grants、data_dir 和取消状态；当前 config_revision 为基线字段，不代表 P3 配置 API 已实现。

SDK 保持读循环与 handler 分离，handler 可以 await Context 调用；start 完成后才调度本插件的业务回调。因此 start 可以调用宿主注册/订阅，但不能等待依赖本插件 action/service/request/event 回调才能完成的调用链，包括经其他插件回到自身的初始化循环。队列、pending 和并发有界。业务自己创建的 tokio::spawn 任务须保留所有权，通过 Context::cancelled 配合停止并在 stop 中 join，SDK 不追踪任意脱离的任务。详见 SDK rustdoc 和 [echo 源码](../../examples/echo-plugin/src/main.rs)。

调试输出使用 stderr，stdout 专用于长度前缀帧。宿主当前消费并丢弃插件 stderr，没有日志展示 API；调试可使用 SDK run_io 测试、断点或显式配置的本地诊断文件。不要独立运行 run_stdio 二进制后期待终端交互；它需要宿主握手。协议故障、缺失 grants、错误 entrypoint 或超时分别参阅[协议](protocol.md)、[清单](manifest.md)、[动作](actions.md)与[排障](troubleshooting.md)。
