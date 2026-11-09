# Rust SDK 公共 API 参考

这份参考回答“我在 Rust 插件里实际能调用什么”。SDK 是一个把 JSON 进程协议包装成异步函数的库，不是宿主本身，也不提供 QQ 登录。插件实现 `Plugin` 回调，把程序交给 `run_stdio`；SDK 在收到宿主消息时调用你的回调，你在回调中用 `Context` 请求宿主。

第一次接触请先读[从零编写第一个插件](first-plugin.md)。本页假设你认识 Rust 的 `async`、`Result` 和 `serde_json::Value`（任意 JSON 值）。除完整程序外，代码片段均放在回调或异步测试中使用，不是独立的可执行文件。

## 参考范围与源码

本页按当前仓库实现核对，而不是按架构规划猜测：

- [SDK 公共类型与 Context](../../crates/twintail-plugin-sdk/src/lib.rs)。
- [运行器实现](../../crates/twintail-plugin-sdk/src/runtime.rs)。
- [配置事务实现](../../crates/twintail-plugin-sdk/src/configuration.rs)。
- [握手和错误类型](../../crates/twintail-protocol/src/lib.rs)、[路由载荷](../../crates/twintail-protocol/src/routing.rs)、[配置载荷](../../crates/twintail-protocol/src/configuration.rs)、[管理载荷](../../crates/twintail-protocol/src/management.rs)。

SDK 的 Cargo 包版本为 `0.1.0`，通过本地源码路径依赖使用；当前 wire 为 `1.2`，SDK 接受协商 `1.1` 或兼容的更新 minor。包版本、线协议版本与配置格式版本是不同维度。`0.1.0` 不表示 Rust API 已有稳定兼容承诺；新项目应使用与宿主相同版本的源码。

SDK 公开的全部自有 API 是：`Result`、`RuntimeOptions`、`Plugin`、`Context`、`PluginReader`、`PluginWriter`、`run_stdio`、`run_io`。另外重导出下文列出的协议载荷与错误类型；没有公开“任意方法名”的原始宿主客户端。

## 运行器：交出 stdin/stdout

```rust
pub async fn run_stdio<P: Plugin>(plugin: P, hello: Hello) -> Result;

pub async fn run_io<P: Plugin, R: PluginReader, W: PluginWriter>(
    plugin: P,
    hello: Hello,
    reader: R,
    writer: W,
    options: RuntimeOptions,
) -> Result;

pub trait PluginReader: AsyncRead + Unpin + Send + 'static {}
pub trait PluginWriter: AsyncWrite + Unpin + Send + 'static {}
```

这里 `AsyncRead` / `AsyncWrite` 来自 Tokio；满足约束的类型已有自动实现，不需为自己的流手写空 trait 实现。

- `plugin`：拥有你的业务状态、实现 `Plugin` 的值。运行器取得所有权。
- `hello`：插件身份与必需权限，必须和包清单一致。
- `run_stdio` 使用进程标准输入和标准输出，以及默认 `RuntimeOptions`。通常作为 `main` 的主要等待任务；不把它当交互命令行程序直接运行。
- `run_io` 接收拥有所有权的异步流，适合用 `tokio::io::duplex` 做协议测试，也适合需要自定 SDK 限额的作者。它不是绕过宿主授权的入口，另一端仍须实现宿主握手和协议。
- 正常关闭且清理成功返回 `Ok(())`。握手失败、超时、非法帧、回调失败等返回 `RpcError`；配置维护请求的可恢复错误通过各次 RPC 返回。
- 丢弃 `run_io` future 会中止它持有的任务并退役等待客户端；不要随后复用可能只读写了半帧的连接。
- 所有 stdout 字节都由运行器独占。不要 `println!`，诊断使用 `eprintln!`；当前宿主读取并丢弃插件 stderr，并没有日志查询 API。

完整启动示例见[第一个插件](first-plugin.md)。底层帧、协议错误与非 Rust 实现见[通信协议](protocol.md)。

## Hello与Initialize

`Hello` 是**你向宿主声明的身份**；`Initialize` 是**宿主实际分配给你的上下文**。前者不是权限凭证，后者也不应由插件自行构造来冒充其他实例。

```rust
pub struct Hello {
    pub plugin_id: String,
    pub version: String,
    pub required_capabilities: Vec<String>,
}

pub struct Initialize {
    pub instance_id: String,
    pub generation: u64,
    pub granted_capabilities: Vec<String>,
    pub data_dir: String,
    pub config_revision: u64,
    pub config: Option<ConfigSnapshot>,
}
```

| 字段 | 如何使用 |
| --- | --- |
| `Hello.plugin_id` | 与清单 `id` 完全相同，不是可随意修改的显示名 |
| `Hello.version` | 与清单包版本相同；常用 `env!("CARGO_PKG_VERSION").into()` |
| `Hello.required_capabilities` | 与清单必需能力集合一致；缺少授予即启动失败 |
| `Initialize.instance_id` | 宿主绑定的实例标识，不是稳定插件 ID |
| `Initialize.generation` | 本次启动代次；重新启动或恢复会更换 |
| `Initialize.granted_capabilities` | 已申请与管理员已授予能力的精确交集 |
| `Initialize.data_dir` | 插件业务数据目录；不要据当前工作目录推算它 |
| `Initialize.config_revision` | 启动时选定的配置修订 |
| `Initialize.config` | 可选启动快照；旧宿主或测试夹具可能不提供；动态更新后仍保持启动值 |

宿主先校验整个包与计划，再校验 hello。收到 initialize 前，插件不知道分配到的实例、代次和数据路径。身份不匹配、版本不兼容或必需权限缺失都可能使运行器返回错误。详见[清单](manifest.md)、[权限](permissions.md)与[生命周期](lifecycle.md)。

## Plugin回调

准确 trait 约束是 `pub trait Plugin: Send + Sync + 'static`。以下是去掉默认函数体后的全部方法签名；实际实现可像 echo 示例一样使用 `async fn`。

```rust
fn start(&self, context: Context) -> impl Future<Output = Result> + Send;
fn event(&self, context: Context, event: EventEnvelope)
    -> impl Future<Output = Result> + Send;
fn action(&self, context: Context, call: ActionCall)
    -> impl Future<Output = Result<Value>> + Send;
fn service(&self, context: Context, call: ServiceCall)
    -> impl Future<Output = Result<Value>> + Send;
fn request(&self, context: Context, method: String, payload: Value)
    -> impl Future<Output = Result<Value>> + Send;
fn prepare_config(&self, context: Context, snapshot: ConfigSnapshot)
    -> impl Future<Output = Result> + Send;
fn commit_config(&self, context: Context, snapshot: ConfigSnapshot)
    -> impl Future<Output = Result> + Send;
fn stop(&self, context: Context) -> impl Future<Output = Result> + Send;
```

`Future` 来自 `std::future::Future`，`Value` 来自 `serde_json`；`Result` 是 SDK 的别名。源码参数用 `_context` 等下划线名称表示默认实现没有使用参数，参数类型与这里相同。

| 回调 | 何时调用、参数和成功含义 | 默认行为与失败 |
| --- | --- | --- |
| `start` | 宿主 activate 后；用 context 注册服务/账号或订阅。成功后才调度业务回调；wire ≥1.2 宿主会等待启动确认 | 默认 `Ok(())`；错误使 runtime 退役 |
| `event` | 匹配订阅的通知到来，event 含来源、时间和 payload。成功只表示本次回调结束，不回给发布者完成确认 | 默认忽略并成功；返回错误终止 runtime，可恢复业务错误应在内部处理 |
| `action` | 适配器收到 `actions.handle`；call 中有目标 bot、动作名与参数；成功 JSON 原样回调用者 | 默认 `Unsupported`，消息 `plugin method unsupported` |
| `service` | 已注册服务收到 `services.handle`；按 call.service/method 分派，返回双方约定的 JSON | 默认同上；返回 RpcError 回给本次调用者 |
| `request` | 宿主调用其他自定义方法，如 mock 测试入口；method 是方法名，payload 是参数 | 默认同上；并非普通插件间任意调用入口，CLI 管理身份与服务权限分离 |
| `prepare_config` | 新配置落盘前验证 snapshot；成功仅表示可准备，不可更改 applied 或产生外部副作用 | 默认 Unsupported；拒绝时不更新 desired |
| `commit_config` | 宿主已落盘后应用准备过的 snapshot；成功后 SDK 才替换 `Context::config()` | 默认 Unsupported；失败保留旧 SDK 快照并清除准备态，宿主可能已持久化 desired |
| `stop` | 业务回调取消并收束后释放插件自建资源；返回成功表示自身清理完成 | 默认成功；受 shutdown_timeout 限制，宿主还可能更早 kill |

回调没有单独的“接收权限”参数：宿主按注册、订阅、目标身份及调用者权限检查路由。`Plugin::action` 不是业务插件发送动作的方法，发送要用 `Context::call_action`；这是新人最容易混淆的两个方向。

回调可能并发，`&self` 不代表串行执行。用合适的锁、原子变量或消息队列管理共享状态；不要跨 await 持有会阻塞线程的同步锁。配置回调之间串行，但可与业务回调并发。`start` 不应等待需要本插件业务回调才能完成的循环调用链。所有 futures 都须让出执行权；长时间同步工作需要有界的阻塞工作线程。

SDK 追踪它调度的回调，不追踪你任意 `tokio::spawn` 的脱离任务。保留任务句柄，让任务响应取消，并在 stop 中 join；连接、监听器与子进程同样需要明确所有权。排空和停止细节见[生命周期](lifecycle.md)。

## Context公共方法

`pub struct Context` 内部字段私有，支持 `Clone`。由 SDK 传入回调；克隆仍共享当前实例的连接、取消状态与动态快照，不产生新的插件身份。

### 本地读取和取消

```rust
pub fn initialize(&self) -> &Initialize;
pub fn config(&self) -> Option<ConfigSnapshot>;
pub fn is_cancelled(&self) -> bool;
pub async fn cancelled(&self);
```

这些方法没有额外 RPC 或能力要求。`initialize` 返回启动上下文的借用；`config` 返回当前已应用快照的克隆，已取得的旧快照不会原地变化。`is_cancelled` 判断关闭/连接退役是否已开始；`cancelled` 等待该信号，多任务可各自等待，返回 `()`，不是新的取消令牌对象。

```rust,ignore
// context 来自 start 或其他回调；后台任务需要保存 JoinHandle 并在 stop 收束。
let worker_context = context.clone();
let worker = tokio::spawn(async move {
    worker_context.cancelled().await;
    // 关闭本任务拥有的资源。
});
```

不要记录整个 initialize 或配置：配置可能含已解析的秘密。数据读写由插件自行实现，见[存储](storage.md)。

### 调用宿主的全部类型化方法

以下所有 `Result` 均以 `RpcError` 失败。权限是清单申请与管理员 grants 交集，方法名与能力名有时不同。

| 准确签名 | 用途与成功值 | 所需能力 |
| --- | --- | --- |
| `pub async fn register_bot(&self, bot: RegisterBot) -> Result<BotRef>` | 适配器为自身实例注册账号；返回可路由的引用 | `bots.register` |
| `pub async fn subscribe(&self, subscription: Subscribe) -> Result<Value>` | 替换完整精确订阅集合；返回宿主确认 JSON，空集合取消订阅 | `events.subscribe` |
| `pub async fn publish(&self, event: PublishEvent) -> Result<DeliveryReport>` | 适配器发布已注册账号的事件；返回准入计数 | `events.publish` |
| `pub async fn call_action(&self, action: ActionCall) -> Result<Value>` | 向 bot 绑定的适配器代次请求动作；返回协议自定义 JSON | `actions.call` |
| `pub async fn register_service(&self, registration: ServiceRegistration) -> Result<Value>` | 注册命名、版本化服务；返回确认 JSON | `services.provide`，对应 wire 方法 `services.register` |
| `pub async fn call_service(&self, call: ServiceCall) -> Result<Value>` | 指定提供者及接口范围调用；返回服务自定义 JSON | `services.call` |
| `pub async fn management(&self, call: ManagementCall) -> Result<Value>` | 调用白名单管理方法，变更通常先返回 operation ACK | 精确 `management.read` 或 `management.write`，二者不互含 |
| `pub async fn health(&self) -> Result<Value>` | 请求 `host.health`，成功 JSON 含状态、插件 ID、generation | `host.health` |

`register_bot` / `publish` 通常属于适配器，回复插件一般只需 subscribe / call_action。服务调用与管理调用不是任意方法转发器，管理白名单不允许借它调用任意插件业务 RPC。完整管理方法、参数和权限见[管理桥接契约](web-admin.md#宿主管理桥接契约)。

所有远程方法还可能因运行器取消、队列/在途耗尽、传输退役、超时或响应格式不符失败。宿主再校验账号归属、名字格式、版本及目标状态。常见 code 及恢复步骤见[错误](errors.md)；返回 `Value` 的方法不应臆造一个 SDK 并未声明的强类型结果。

### 各种调用共享的超时规则

每次请求生成唯一 ID，一个 deadline 覆盖“等待进入发送队列 + 等待响应”。future 被丢弃会移除本地 pending 项，不会撤销远端已接管的操作。没有 SDK 自动重试或自动重新连接。

SDK 在动作已经进入自己的请求队列后，若本地发生超时、取消或连接退役，会保守地返回 `UnknownOutcome`；入队之前的本地失败仍用原类别。宿主也会对已派发后丢失结果的动作报告结果未知。发送消息等副作用动作不要自动重试。非动作调用的 Timeout/Cancelled 同样不保证服务或管理副作用被撤销。

## 重导出的协议载荷

以下是 SDK 的全部协议重导出，字段与源码一致。它们是可序列化数据，不会仅因构造成功就通过宿主语义校验。

### BotRef：账号加适配器实例 {#botref}

```rust
pub struct BotRef {
    pub adapter_instance_id: String,
    pub self_id: String,
}
pub struct RegisterBot {
    pub self_id: String,
}
```

`self_id` 是字符串账号，避免数值精度丢失；`adapter_instance_id` 是宿主分配、绑定代次的实例标识，不是稳定插件 ID。业务插件使用 `event.bot`，适配器使用 register_bot 返回值。不要仅按账号构造路由，也不要在重载后复用旧 BotRef；新事件会带新引用。

### 事件的订阅、发布与接收

```rust
pub struct Subscribe {
    pub event_types: Vec<String>,
}
pub struct PublishEvent {
    pub self_id: String,
    pub protocol: String,
    pub event_type: String,
    pub payload: Value,
}
pub struct EventEnvelope {
    pub event_id: String,
    pub bot: BotRef,
    pub received_at_ms: u64,
    pub protocol: String,
    pub event_type: String,
    pub payload: Value,
}
pub struct DeliveryReport {
    pub delivered: u32,
    pub dropped: u32,
}
```

`event_types` 精确匹配 event_type，无通配符；protocol 是内容的命名空间，不是自动筛选条件。发布只能使用本实例注册过的 self_id。宿主填入 event_id、bot 与 Unix 毫秒接收时间。payload 保留消息段和扩展 JSON；event_id 是宿主关联 ID，不是上游消息去重键。

`delivered` 统计进入订阅者队列的数量，`dropped` 统计匹配订阅者准入失败的数量，不表示回调完成或消息发送。没有订阅者时也可能成功返回。完整发布示例、队列 32 及命名限制见[事件](events.md)。

### 动作和服务

```rust
pub struct ActionCall {
    pub bot: BotRef,
    pub action: String,
    pub params: Value,
}
pub struct ServiceRegistration {
    pub name: String,
    pub version: String,
}
pub struct ServiceCall {
    pub provider: String,
    pub service: String,
    pub version_req: String,
    pub method: String,
    pub params: Value,
}
```

ActionCall 没有 timeout、deadline 或调用 ID 字段；超时由运行器和宿主控制。协议动作名与 params 由适配器定义，完整 `send_msg` 参数复制例子见[第一个插件](first-plugin.md)。

ServiceCall 的 provider 是稳定插件 ID，service 是该提供者注册的精确名字，version_req 匹配**服务接口**版本而非包版本，method/params 是双方约定的接口。宿主在每次调用时解析提供者当前代次并固定路由，不把在途请求转给新代。注册/调用/处理完整示例和错误见[服务](services.md)。

### 配置与管理

```rust
pub struct ConfigSnapshot {
    pub config_version: u32,
    pub revision: u64,
    pub values: Value,
}
pub struct ConfigPrepare {
    pub expected_revision: u64,
    pub snapshot: ConfigSnapshot,
}
pub struct ConfigCommit {
    pub revision: u64,
}
pub struct ManagementCall {
    pub method: String,
    pub payload: Value,
}
```

配置格式版本由插件定义，revision 由宿主维护。ConfigPrepare / ConfigCommit 是协议数据，由 SDK 运行器处理，不是需要业务插件手动调用的 Context 方法。`config.abort {revision}` 和 `config.status` 也由运行器处理；没有 `Plugin::abort_config` 回调。prepare 无副作用，commit 回调期间 `context.config()` 仍是旧值；成功后才切换。已准备候选只有一个；落盘失败时宿主 abort，无候选时幂等成功，不同候选修订返回 Conflict。

ManagementCall 反序列化拒绝未知字段。payload 是该白名单方法的 JSON 对象；配置管理读取返回秘密引用，不返回解析值。变更 ACK 只是受理，须查询 operations；取消等待不撤销变更。配置全过程和离线更新见[配置](configuration.md)。

## Result、RpcError和ErrorCode

```rust
pub type Result<T = ()> = std::result::Result<T, RpcError>;
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}
// RpcError 的关联构造方法：
pub fn new(code: ErrorCode, message: impl Into<String>) -> Self;
```

`RpcError` 实现 Display 和标准 Error。ErrorCode 全部变体为：`IncompatibleVersion`、`PermissionDenied`、`Offline`、`Timeout`、`UnknownOutcome`、`Cancelled`、`Overloaded`、`InvalidConfig`、`PluginExited`、`ReloadFailed`、`RemoteError`、`InvalidFrame`、`InvalidRequest`、`DependencyFailed`、`NotFound`、`Conflict`、`Unsupported`。wire 使用对应 snake_case。错误 message 不稳定，不用于程序分支，也不要放秘密、完整配置或敏感消息。逐项处理建议见[错误](errors.md)。

## RuntimeOptions：SDK限额不等于宿主限额

类型派生 `Clone, Debug`，实现 Default。全部字段如下：

| 字段与 Rust 类型 | 默认值 | 作用 |
| --- | --- | --- |
| `queue_capacity: usize` | 64 | 各独立 reader/request/response/control/event 队列容量 |
| `max_handlers: usize` | 16 | 最大并发回调数，含激活 |
| `max_pending: usize` | 128 | 插件到宿主的最大等待调用数 |
| `max_request_ids: usize` | 65536 | SDK 保留的不同入站请求 ID 预算 |
| `request_timeout: Duration` | 10 秒 | 每次出站宿主调用总期限 |
| `handshake_timeout: Duration` | 10 秒 | 整个握手期限 |
| `shutdown_timeout: Duration` | 3 秒 | stop 回调期限 |
| `drain_timeout: Duration` | 30 秒 | drain 等待已运行回调的期限 |

四个容量/计数字段设为 0 时，运行器返回 `InvalidConfig`，消息 `runtime limits must be positive`。配置示例：

```rust,ignore
use std::time::Duration;
use twintail_plugin_sdk::RuntimeOptions;
let options = RuntimeOptions {
    max_handlers: 4,
    request_timeout: Duration::from_secs(5),
    ..RuntimeOptions::default()
};
// 传给 run_io(plugin, hello, reader, writer, options).await
```

调高 SDK 数值不会调高宿主硬上限：宿主事件队列每实例 32、插件请求 ID 每连接累计 4096、默认请求期限 5 秒；更早的那一端可能先失败。宿主 ID 预算与 SDK `max_request_ids` 分别约束不同方向，不要混为一个可调参数。队列有界也不代表整个进程的 CPU/RSS 已有硬限制。完整宿主配额见[协议](protocol.md)。

## 当前没有的 API 与验证入口

当前没有结构化日志 SDK、任意存储读写 RPC、任意宿主请求入口、自动重连/重试、事件持久重放、通配订阅或 parallel_safe 重载实现。不要把架构目标当作已可调用的函数。OneBot 类型库是独立 crate，适配器网络重连也不是 SDK 自动行为。

需要验证你自己的包时，使用[契约检查工具](contract-tools.md)；SDK 与宿主的历史设计见 [ADR 0003](../architecture/0003-p2-sdk-routing.md)和 [ADR 0004](../architecture/0004-p3-management-config-reload.md)。平台与真实外部服务的已验证范围分别见[平台矩阵](../release/platforms.md)和[SnowLuma 兼容矩阵](../compatibility/snowluma.md)。
拓展阅读：[配置](configuration.md) · [权限](permissions.md) · [排障](troubleshooting.md)。
