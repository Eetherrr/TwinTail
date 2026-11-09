# 第一次运行：用假消息测试插件

本页带你从源码构建 TwinTail，启动两个示例插件，再手动输入一条假消息。最后你会看到一条“发送回复”的动作记录。**不需要 QQ 账号、不需要 SnowLuma，也不会向外部发送消息。**

## 先认识演示里的角色

- **宿主** `twintail-host`：启动插件并在它们之间转发事件和动作。
- **mock-adapter**：模拟机器人接入端。这里的 mock 就是“假的测试替身”；你让它产生消息，它也负责记录插件想发送的回复。
- **echo-plugin**：收到什么文本，就请求回复相同文本的示例业务插件。
- **包与清单**：包是一个目录，里面放可执行文件和 `plugin.toml`；清单告诉宿主插件叫什么、从哪里启动、需要哪些权限。
- **数据根与安装根**：数据根放运行配置和业务数据；安装根放插件程序。分开存，升级程序时才不会顺手覆盖数据。

本次数据走向：你输入假消息 → mock → 宿主 → echo → 宿主 → mock 的动作记录。看到记录就完成了本教程；这与真实 QQ 收件是两回事。

## 开始前准备

1. 取得完整 TwinTail 源码，在仓库根目录打开终端（这里能看到 `Cargo.toml` 和 `rust-toolchain.toml`）。
2. 安装 Rust/rustup，并使用仓库固定的 **Rust 1.99.1** 与 `Cargo.lock`。首次构建需要下载对应工具链和依赖，也需要系统链接器。
3. 以下主流程使用 Bash/Linux 风格命令；准备两个终端，一个持续运行宿主，另一个发管理命令。Windows 的区别见后文。
4. 使用新的演示目录，不要指向现有正式数据。命令中的 `$DEMO` 是本次运行的临时目录，不是固定安装位置。

### 已下载预编译包怎么办？

预编译归档里已经有宿主和版本化插件包，可以跳过编译，按[部署说明](../release/operations.md)使用其实际目录。**本页的复制命令面向完整源码**，不是让你在预编译包中运行 Cargo。归档采用 `packages/<id>/<version>/`，本教程刻意采用宿主仍支持的静态 `packages/<id>/` 布局，减少第一次运行的步骤并演示旧静态计划的兼容路径；这不表示当前版本没有安装、配置或重载能力。

## 第 1 步：构建并组装两个包

在仓库根执行，保留当前终端供后续命令使用。先用 `pwd` 确认所在目录；构建完成后会打印演示目录绝对路径，请记下它：

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

## 第 2 步：启动宿主

启动已打包宿主并保持前台运行。终端不返回提示符是正常的，表示它正在等待消息：

```sh
"$DEMO/bin/twintail-host" \
  --data-dir "$DEMO/data" --install-dir "$DEMO/packages"
```

## 第 3 步：发送假消息并查看回复

在第二个终端将 DEMO 设置为第 1 步打印的实际绝对路径，然后运行：

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

## 第 4 步：停止并退出

先停止依赖者 echo，再停止它依赖的 mock，或在宿主终端按 Ctrl-C 关闭所有实例：

```sh
"$DEMO/bin/twintail-host" plugins stop echo-plugin --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins stop mock-adapter --data-dir "$DEMO/data"
```

停止被活动 echo 依赖的 mock 会得到 conflict。本教程刻意使用兼容的静态打包路径；当前版本的安装、配置事务与重载分别见[存储](storage.md)、[配置](configuration.md)与[热重载](hot-reload.md)。已启用但停止的实例可使用 plugins start，重新启动后 BotRef 属于新 generation，不复用旧实例路由。

## Windows 使用说明

Windows 构建产物包含 `.exe`。在 PowerShell 的新目录构建与复制时，对三个 bin 使用 `twintail-host.exe`、`echo-plugin.exe`、`mock-adapter.exe`；两个包内 plugin.toml 的 entrypoint 也分别改为 `echo-plugin.exe`、`mock-adapter.exe`。插件 id、包目录名与 capabilities 保持示例原值。使用 `New-Item -ItemType Directory` 创建目录、`Copy-Item` 复制文件，以 `& "$demo\bin\twintail-host.exe" ...` 调用。设置与 Unix 示例等价的 host.toml 后，先 --validate，再前台启动；管理使用宿主实现的本机命名管道，不创建 management.sock。JSON 仍必须作为单参数；PowerShell 可用 `'{"message":[{"type":"text","data":{"text":"hello"}}]}'`，并按所用 PowerShell 版本的原生命令参数规则传递。Windows 原生 CI 已通过，范围见[平台矩阵](../release/platforms.md)；本页 PowerShell 手工流程与真实 QQ 不因此被认定已专项验收。

## 接下来：编写自己的插件

编写自己的插件时实现 SDK 的 `Plugin` trait，并调用 `run_stdio(plugin, Hello { ... })`。Hello 的 plugin_id、version、required_capabilities 须与清单一致；`Plugin::start` 在宿主 activate 后注册机器人/服务或订阅，`event`、`action`、`service` 分别处理三类业务。Context 提供宿主初始化身份、grants、data_dir、取消状态以及 `config()` 配置快照。当前版本已实现配置事务、desired/applied 修订与 prepare/commit/abort 回调，具体约定见[配置指南](configuration.md)；本页示例不演示配置更新。

SDK 保持读循环与 handler 分离，handler 可以 await Context 调用；start 完成后才调度本插件的业务回调。因此 start 可以调用宿主注册/订阅，但不能等待依赖本插件 action/service/request/event 回调才能完成的调用链，包括经其他插件回到自身的初始化循环。队列、pending 和并发有界。业务自己创建的 tokio::spawn 任务须保留所有权，通过 Context::cancelled 配合停止并在 stop 中 join，SDK 不追踪任意脱离的任务。详见 SDK rustdoc 和 [echo 源码](../../examples/echo-plugin/src/main.rs)。

调试输出使用 stderr，stdout 专用于长度前缀帧。宿主当前消费并丢弃插件 stderr，没有日志展示 API；调试可使用 SDK run_io 测试、断点或显式配置的本地诊断文件。不要独立运行 run_stdio 二进制后期待终端交互；它需要宿主握手。协议故障、缺失 grants、错误 entrypoint 或超时分别参阅[协议](protocol.md)、[清单](manifest.md)、[动作](actions.md)与[排障](troubleshooting.md)。


## 成功标志与常见问题

| 看到什么 | 意味着什么 / 接下来怎么做 |
| --- | --- |
| `--validate` 成功 | 包和启用计划通过离线检查，尚未启动插件；不证明配置秘密可解析或插件业务可用 |
| 两个 `ready` 探针均为 `value.ready = true` | 示例初始化完成，可以注入消息 |
| `mock.emit` 的 `delivered=1, dropped=0` | 消息进入 echo 队列，不代表已处理 |
| `mock.actions` 中出现 `send_msg` 和 `hello TwinTail` | 本地收消息→请求回复链路已完成 |
| 找不到可执行文件 | 检查复制命令、包内入口名和 Windows `.exe` 后缀；不要把源码目录当运行包 |
| 连接本机管理端点失败 | 第一个终端的宿主是否仍在运行？两个终端的 `DEMO` 是否同一个绝对路径？ |
| `permission_denied` 或缺能力 | 核对 `host.toml` 中 grants；清单申请权限不等于宿主授予权限 |
| 动作列表暂时为空 | 稍后只重复 `mock.actions` 查询，不重复注入消息 |
| 停止 mock 得到 conflict | 先停止依赖它的 echo |

### 版本与验证范围（需要时再查）

SDK、protocol、core、host 与示例的 Cargo 包版本为 `0.1.0`；wire 为 `1.2`。清单 `protocol_version = 1` 是 wire 主版本，`host_api = "^0.1.0"` 是宿主 API 兼容范围。SDK 接受 wire 1.1 或兼容的更高 minor，低于 1.1 拒绝初始化；配置和应用启动确认扩展需双方支持 1.2。能力也参与握手校验，major 相同不代表旧宿主支持所需路由。SDK 使用 JSON IPC，不依赖跨进程 Rust ABI。

2026-10-29 Linux 本地已用本页包结构完成独立 release 构建、两包离线校验、CLI 注入/查询及顺序停止：`delivered=1/dropped=0`，动作记录含相同消息段，宿主正常退出。这是该演示的历史专项记录，非本次重新运行结论；最新原生 CI 见[平台矩阵](../release/platforms.md)，完整历史见[兼容性记录](../compatibility/README.md)。Linux 风格命令不代表 macOS 或每种 PowerShell 环境均已专项验证。
