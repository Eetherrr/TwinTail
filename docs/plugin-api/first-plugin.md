# 从零编写第一个 Rust 插件

目标是自己写一个完整程序：收到 `message` 事件后，把消息内容和会话目标复制到 `send_msg` 动作，让来源适配器执行。先用 mock 适配器记录动作，无需 QQ 账号。这与仓库 [echo 实现](../../examples/echo-plugin/src/main.rs)的消息处理一致，不额外发明 `send_text` 一类 SDK 方法。

## 先建立一个简单模型

- **宿主 host**：启动和监管程序、校验权限、转交事件与动作。
- **适配器 adapter**：连接外部协议服务，向宿主报告“收到了消息”，接收“发送消息”的动作。教程使用的 mock 只在内存记录动作，不连接 QQ。
- **业务插件**：本教程编写的回复程序，不处理 QQ 登录和 WebSocket。
- **SDK**：你依赖的 Rust 库。它读取宿主的 stdin 消息，调用你的 `Plugin` 方法，再把 `Context` 请求写到 stdout。
- **事件 event / 动作 action**：一个是“发生了什么”的通知，一个是“请做什么”的请求。

程序编译成功后，还要和 `plugin.toml` 一起打包，再由管理员在 `host.toml` 中启用并授予权限。Cargo.toml 是编译配置；plugin.toml 是运行清单，两者不互相替代。

## 1. 准备源码和工具链

需要同版本 TwinTail 源码、仓库固定的 Rust `1.99.1` 工具链、Cargo，以及能下载 Rust 依赖或已有缓存的环境。SDK 当前没有发布到 registry 的使用承诺，本教程明确使用源码路径依赖，而非 `twintail-plugin-sdk = "0.1"`。

以下是 Linux/Unix shell 流程，先在 TwinTail 仓库根执行。用一个全新的目录保存自己的程序，避免改动 workspace 成员配置：

```sh
REPO="$PWD"
DEMO="$(mktemp -d "${TMPDIR:-/tmp}/twintail-first.XXXXXX")"
export REPO DEMO
mkdir -p "$DEMO/hello-plugin/src"
```

将下面内容保存为 `$DEMO/hello-plugin/Cargo.toml`，把路径占位符换成 `REPO` 的**实际绝对路径**。Cargo.toml 不会展开 shell 的 `$REPO`。若路径含 TOML 特殊字符，请按 TOML 字符串规则转义。

```toml
[package]
name = "hello-plugin"
version = "0.1.0"
edition = "2024"
rust-version = "1.99.1"
publish = false

[dependencies]
twintail-plugin-sdk = { path = "/absolute/path/to/TwinTail/crates/twintail-plugin-sdk" }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt", "io-std", "io-util", "sync", "time"] }
```

独立项目在仓库外，不使用 `version.workspace = true` 等继承字段。SDK 自身仍在原仓库，Cargo 能找到其 workspace 元数据；不要只复制 SDK 的 src 目录。要固定独立项目依赖版本，保留首次构建生成的 Cargo.lock。

## 2. 写出完整入口与回调

将下面**整段**保存为 `$DEMO/hello-plugin/src/main.rs`：

```rust
use serde_json::{Map, Value};
use twintail_plugin_sdk::{
    ActionCall, Context, EventEnvelope, Hello, Plugin, Result, Subscribe, run_stdio,
};

struct HelloPlugin;

impl Plugin for HelloPlugin {
    async fn start(&self, context: Context) -> Result {
        context
            .subscribe(Subscribe {
                event_types: vec!["message".into()],
            })
            .await?;
        Ok(())
    }

    async fn event(&self, context: Context, event: EventEnvelope) -> Result {
        let mut params = Map::new();
        for field in ["message", "message_type", "user_id", "group_id"] {
            if let Some(value) = event.payload.get(field) {
                params.insert(field.into(), value.clone());
            }
        }
        if !params.contains_key("message") {
            return Ok(());
        }
        context
            .call_action(ActionCall {
                bot: event.bot,
                action: "send_msg".into(),
                params: Value::Object(params),
            })
            .await?;
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = run_stdio(
        HelloPlugin,
        Hello {
            plugin_id: "hello-plugin".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            required_capabilities: vec!["events.subscribe".into(), "actions.call".into()],
        },
    )
    .await;
    if let Err(error) = result {
        eprintln!("hello-plugin: {error}");
        std::process::exit(1);
    }
}
```

逐段理解：

1. `HelloPlugin` 是插件业务状态。此例无须保存状态，所以是空结构；有状态时回调会并发，应显式同步。
2. `impl Plugin` 实现宿主可调用的回调。`start` 不是操作系统进程入口，而是宿主允许激活后执行的初始化。这里订阅精确的 `message` 类型；每次订阅会替换完整集合。
3. `Context` 是 SDK 传给你的当前实例客户端，不要自己构造。`subscribe` 和 `call_action` 是向宿主发请求的异步方法，`.await?` 表示等待结果，遇到错误向上返回。
4. `EventEnvelope` 是宿主包装后的事件。`payload` 内是协议 JSON；本例只复制发消息需要的四个字段，既保留原始消息段，也不把任意扩展误当发送参数。
5. `event.bot` 是 `BotRef`，包含来源适配器实例和字符串账号。原样用于回复，确保路由到收到这条消息的同一代适配器；不要用 QQ 号或稳定插件 ID 替代它。
6. `ActionCall` 的三个字段是 `bot`、`action`、`params`。`send_msg` 是 OneBot 动作名，不是 Context 的函数；返回的是适配器定义的 JSON。mock 只记录它，真实发送由适配器实现。
7. `main` 先创建 Tokio 异步运行环境，再等待 `run_stdio`。`Hello` 的身份、版本和必需权限必须与下面清单一致。SDK 自动完成 hello/negotiate/initialize/ready/activate 通信，不需要手工写 JSON 帧。
8. 所有日志写 stderr。stdout 是协议专线，`println!` 会破坏连接。当前宿主丢弃插件 stderr；需详细调试时使用 [run_io 测试](sdk-reference.md)或显式的本地诊断文件，不期待 WebUI 显示 stderr。

这段程序保持 echo 的简单错误行为：动作失败会通过 `?` 使 event 回调失败，进而终止 SDK runtime。生产业务如需继续运行，应在回调内按[错误类别](errors.md)处理可恢复错误；`UnknownOutcome` 意味可能已发送，别自动重试。

## 3. 构建和打包

仍在仓库根执行；构建工具链显式固定，独立项目第一次会生成自己的锁文件：

```sh
cargo +1.99.1 build --manifest-path "$DEMO/hello-plugin/Cargo.toml" \
  --target-dir "$DEMO/build-plugin"
cargo build --locked -p twintail-host -p mock-adapter \
  --target-dir "$DEMO/build-host"
mkdir -p "$DEMO/bin" "$DEMO/packages/hello-plugin" \
  "$DEMO/packages/mock-adapter" "$DEMO/data"
chmod 700 "$DEMO/data"
cp "$DEMO/build-plugin/debug/hello-plugin" "$DEMO/packages/hello-plugin/"
cp "$DEMO/build-host/debug/mock-adapter" "$DEMO/packages/mock-adapter/"
cp "$DEMO/build-host/debug/twintail-host" "$DEMO/bin/"
cp "$REPO/examples/mock-adapter/plugin.toml" "$DEMO/packages/mock-adapter/"
```

保存下面内容为 `$DEMO/packages/hello-plugin/plugin.toml`：

```toml
id = "hello-plugin"
name = "My first reply plugin"
version = "0.1.0"
protocol_version = 1
host_api = "^0.1.0"
entrypoint = "hello-plugin"
capabilities = ["events.subscribe", "actions.call"]
required_capabilities = ["events.subscribe", "actions.call"]

[dependencies]
mock-adapter = "^0.1.0"
```

清单 `protocol_version = 1` 是线协议主版本，不是 Cargo 包版本；SDK 当前发送 wire 1.2。`host_api` 是宿主 API 版本范围。`entrypoint` 是包内可执行文件相对路径，不是 Rust 源码路径。声明 mock 强依赖保证先启动 mock；以后改用真实适配器时应相应调整依赖、包和配置，不把教程依赖照搬到生产。

运行包最终只需这些内容，不依赖构建目录：

```text
packages/
  hello-plugin/
    plugin.toml
    hello-plugin
  mock-adapter/
    plugin.toml
    mock-adapter
```

本教程先用静态包路径。版本安装、摘要校验、卸载与数据保留见[存储指南](storage.md)；不要手改已生成摘要的安装包。

## 4. 显式启用并验证

保存下面内容为 `$DEMO/data/host.toml`：

```toml
[[plugins]]
path = "mock-adapter"
grants = ["bots.register", "events.publish", "services.provide"]

[[plugins]]
path = "hello-plugin"
grants = ["events.subscribe", "actions.call"]
```

`path` 相对 `--install-dir`；`grants` 是管理员授权，单在清单申请不够。缺少必需授权时宿主会拒绝激活，而非悄悄忽略功能。离线验证不会启动插件：

```sh
"$DEMO/bin/twintail-host" --validate \
  --data-dir "$DEMO/data" --install-dir "$DEMO/packages"
printf 'Demo directory: %s\n' "$DEMO"
"$DEMO/bin/twintail-host" \
  --data-dir "$DEMO/data" --install-dir "$DEMO/packages"
```

最后一条保持前台运行。在第二个终端，把 DEMO 设置为打印出的实际路径：

```sh
DEMO=/tmp/twintail-first.替换为实际目录
"$DEMO/bin/twintail-host" plugins status hello-plugin --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.ready '{}' \
  --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.emit \
  '{"message_type":"private","user_id":"10002","message":[{"type":"text","data":{"text":"hello TwinTail"}}]}' \
  --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins call mock-adapter mock.actions '{}' \
  --data-dir "$DEMO/data"
```

先确认 hello-plugin 为 active 且无启动错误、mock.ready 的 ready 为 true。当前 wire 1.2 宿主还会确认 SDK start 成功；旧协议只完成握手不足以证明业务准备好，见[生命周期](lifecycle.md)。

预期结果：mock.emit 返回 `delivered = 1`、`dropped = 0`，随后 mock.actions 的数组中出现一条 `action = "send_msg"`，params 含相同 message 消息段和 user_id。若首次查询为空，可稍后**只重新查询** mock.actions；publish 成功只是入队，事件回调还可能正在执行，重复 mock.emit 会制造第二条动作。

这证明“事件→自己的回调→动作→mock 记录”本地闭环，不证明已发到 QQ。`plugins call` 是本机管理员调用测试入口，不是普通插件绕过服务权限的办法。真实连接请继续读[适配器指南](adapters.md)与[真实兼容矩阵](../compatibility/snowluma.md)。

## 5. 停止，以及接下来该加什么

在管理终端先停依赖者，再停 mock；或在宿主终端按 Ctrl-C：

```sh
"$DEMO/bin/twintail-host" plugins stop hello-plugin --data-dir "$DEMO/data"
"$DEMO/bin/twintail-host" plugins stop mock-adapter --data-dir "$DEMO/data"
```

本例没有后台任务或连接，默认 stop 已够用。以后自己 spawn 的任务须保存句柄、响应 `Context::cancelled().await` 并在 `Plugin::stop` 中 join；SDK 不追踪任意脱离任务。宿主只监管直接子进程，不要派生守护后代。

需要设置时，使用 `context.config() -> Option<ConfigSnapshot>` 获取当前已应用的独立快照；无快照时是 None，不等于读取失败。`context.initialize().config` 仅是启动初值，在线更新后不变。支持在线更新还需实现无副作用 `prepare_config` 和实际应用的 `commit_config`，默认均 Unsupported；详细事务、秘密引用与 desired/applied 见[配置指南](configuration.md)。不要在初学阶段照着架构目标猜测 SDK 有文件存储或日志 API。

下一步阅读：[SDK 完整参考](sdk-reference.md) · [服务调用](services.md) · [错误处理](errors.md) · [包契约检查](contract-tools.md) · [热重载](hot-reload.md)。

## Windows 与验证范围

Windows 可执行文件带 `.exe`，复制时使用 `hello-plugin.exe`、`mock-adapter.exe`、`twintail-host.exe`，两份包清单的 entrypoint 也需对应修改。PowerShell 用自己的目录/复制命令，并保证 JSON 作为单一参数传递；本页 shell 命令不是 PowerShell 脚本。管理走本机受限命名管道，不手工创建 management.sock。

本页完整 Rust 代码与 Cargo 配置已从 Markdown 提取为仓库外临时项目，以当前本地源码进行离线 `cargo check` / `cargo build`。清单与启用计划也从本文提取，Linux 本地宿主验证了 2 包离线校验、hello-plugin active、mock.ready、单次 emit 的 delivered=1/dropped=0、恰好一条保留消息段与目标的 send_msg、顺序停止和宿主退出码 0。这个结果不包括 PowerShell 手工流程或真实 QQ。

2026-11-09 已在 Linux 从本文提取完整 Cargo.toml、main.rs 和两份运行 TOML 到仓库外目录，独立构建通过；使用宿主与 mock 验证 `delivered=1/dropped=0`，恰好记录一条 `send_msg` 且消息和目标一致，停止后宿主退出码为 0。此项不连接真实 QQ，也未验证本文的 PowerShell 手工流程。

本教程使用与 echo 一致的 SDK 调用和 payload 复制逻辑；平台 CI 范围见[平台矩阵](../release/platforms.md)。历史 echo/mock 专项验收见[快速开始](quickstart.md)，不能把 mock、编译通过或协议兼容当作所有平台与真实 QQ 的验证。
