# OneBot 适配器作者指南

P4 已完成 Linux 本地验收及指定真实部署范围验证：Linux x64 WSL mirrored 客户端对 SnowLuma 2.2.1-node 的查询/私聊，以及真实重连后 Supervisor→独立 adapter→feature 单次私聊闭环通过，用户确认收件。远端 OS/commit/SHA未知，其余真实能力按兼容矩阵保留待测。`crates/twintail-onebot11` 提供可复用 OneBot v11 类型，`adapters/adapter-snowluma` 是通过 SDK 与宿主通信的独立插件。宿主不链接该适配器，也不处理 QQ 登录或 SnowLuma 私有管理接口。设计依据见 [ADR 0005](../architecture/0005-p4-onebot-snowluma.md)。

## 连接与身份

P6已实现两种Universal WS传输：forward_ws为默认，适配器作为ws/wss客户端；reverse_ws由适配器在数字loopback监听，OneBot端作为客户端连接。同一连接接收事件和动作响应，split `/api`/`/event` 与HTTP仍延期。使用实际部署配置的地址与路径，不把 OneBot 规范中的端口作为 SnowLuma 默认值。认证通过 Authorization Bearer，秘密采用[配置环境引用](configuration.md)，不能放 URL 查询参数、源码或管理响应。

self_id 配置为字符串。OneBot 边界的整数标识按无精度损失方式接受，不先转浮点数；注册 `BotRef` 时使用宿主分配的 adapter instance 加配置 self_id。入站事件必须匹配该账号，不能从事件自动扩展机器人列表。WS connection generation 用于 echo/pending 隔离，与宿主进程 generation 分别维护；重连不意味着旧调用可迁移。正向模式失败或短连接后退避翻倍至 reconnect_max_ms；连接持续至少 reconnect_max_ms 才将退避复位 initial。当前没有 jitter 和总重试次数预算，对外部 WS 持续重连直到 stop；这不表示宿主自动重启插件进程。

当前配置键按 adapter config.rs 核对如下；未知字段拒绝，所有变更要求实例重启。self_id必填；forward_ws要求url，reverse_ws要求url省略或空字符串。access_token可省略。旧config_version1配置无需转换，缺失transport继续使用forward_ws，不自动切换reverse。

| 字段 | 默认值 | 范围/语义 |
| --- | --- | --- |
| transport | forward_ws | forward_ws或reverse_ws |
| url | 正向必填 | ws/wss，最多4096字节，禁止用户密码、query、fragment；反向省略或空 |
| reverse_bind | 127.0.0.1:17281 | 数字loopback套接字地址，非loopback即使有token也拒绝 |
| reverse_path | /onebot/v11 | 精确路径，不带query |
| self_id | 必填 | 1–128位十进制字符串，多位时无前导0 |
| access_token | 无 | 非空可见ASCII，最多4096字节，仅用于Bearer |
| action_timeout_ms | 3000 | 10–30000，从准入到远端回复 |
| connect_timeout_ms | 3000 | 10–10000，握手及单次写期限 |
| reconnect_initial_ms | 100 | 10–60000 |
| reconnect_max_ms | 10000 | 不小于initial，最多60000 |
| ping_interval_ms | 15000 | 10–60000 |
| pong_timeout_ms | 10000 | 10–60000 |
| max_pending | 64 | 1–1024，排队与在途动作总额 |
| event_queue | 64 | 1–4096，独立发布队列 |
| max_frame_bytes | 1048576 | 1024–4194304，入/出站WS消息 |

包清单声明 config_version = 1、reload_policy = "exclusive"，Schema 位于包内 config.schema.json；Schema 的 default 用作说明，Rust 配置反序列化应用这些缺省值。URL完整约束、token可见ASCII及 reconnect_max ≥ initial 由插件再次校验，不能只凭 Schema 验证成功认定可启动。wss 须验证服务端证书，不使用公开资料查询时的临时证书处理方式连接业务服务。

## 反向Universal WS接入

P6本地mock与SDK独立子进程闭环已通过，包含同端口重载重绑定；不将其计作真实SnowLuma反向验收。首次私有预置配置仍使用下文安装流程，但完整envelope改为以下内容，**移除正向url**：

```toml
config_version = 1
revision = 1

[values]
transport = "reverse_ws"
reverse_bind = "127.0.0.1:17281"
reverse_path = "/onebot/v11"
self_id = "10001"

[values.access_token]
"$secret" = "TWINTAIL_ONEBOT_TOKEN"
```

宿主启动前设置TWINTAIL_ONEBOT_TOKEN；无认证场景删除整个access_token表，账号替换为部署实际值。把OneBot端配置为连接ws://127.0.0.1:17281/onebot/v11的Universal客户端；其握手必须各有唯一X-Self-ID（精确匹配配置字符串）及X-Client-Role: Universal。配置token时须唯一Authorization: Bearer TOKEN。token错401，账号/role/path错403；不接受重复头与query。

仅一条活动socket；额外连接拒绝，通常409（best-effort try_write后drop，不承诺客户端必收到响应）。监听在spawn时bind，端口冲突InvalidConfig；stop会join并释放监听，同端口reload须等旧实例退出。反向模式等待客户端重连，成功连接generation递增；在途断线/超时仍UnknownOutcome、不重放。

现有实例切换：先plugins stop并查询完成，读取当前revision，用config update提交完整values（transport/reverse_bind/reverse_path/self_id及可选秘密引用，不含url），查询完成后plugins start。不要在线编辑envelope或手工将revision置1；可用exclusive reload替换同模式版本，仍有事件缺口且不回滚业务数据。无native TLS或公网绑定，远程仅通过外部SSH隧道映射到loopback，部署方负责隧道认证。

## 构建与首次启用

以下为 Bash/Linux 流程，使用已有 Rust 工具链，从仓库构建到新目录。包内必须同时包含可执行文件、plugin.toml 和 config.schema.json；仅复制源码目录不能加载。Linux loopback CLI 已按私有预置配置→install/enable→desired/applied 1→connected，以及 SnowLuma 专用清单 echo 的 message/send_msg/response 闭环验收；该 mock 证据不证明真实 SnowLuma 兼容。

```sh
REPO="$PWD"
P4_DEMO="$(mktemp -d)"
mkdir -p "$P4_DEMO/bin" "$P4_DEMO/source/adapter-snowluma" "$P4_DEMO/data/plugins/adapter-snowluma" "$P4_DEMO/packages"
chmod 700 "$P4_DEMO/data" "$P4_DEMO/data/plugins" "$P4_DEMO/data/plugins/adapter-snowluma"
cargo build --release --target-dir "$P4_DEMO/build" -p twintail-host -p adapter-snowluma
cp "$P4_DEMO/build/release/twintail-host" "$P4_DEMO/bin/"
cp "$P4_DEMO/build/release/adapter-snowluma" "$P4_DEMO/source/adapter-snowluma/"
cp "$REPO/adapters/adapter-snowluma/plugin.toml" "$REPO/adapters/adapter-snowluma/config.schema.json" "$P4_DEMO/source/adapter-snowluma/"
```

当前 CLI 首次配置存在引导限制：enable 前校验必填配置，而 config 管理只接受启用计划中的插件。对于尚未运行、尚无配置文件的新数据根，先私有预置以下 `data/plugins/adapter-snowluma/config.toml`，再启动宿主。这里的地址和账号是本地示例，替换成被测部署实际值；没有鉴权时删除整个 `[values.access_token]` 表。正式 token 不直接写入该文件。

```toml
config_version = 1
revision = 1

[values]
url = "ws://127.0.0.1:6700/"
self_id = "10001"

[values.access_token]
"$secret" = "SNOWLUMA_ACCESS_TOKEN"
```

创建文件时使用 `umask 077`，使其权限为0600；不要原地覆盖运行实例配置。启动宿主前在私有终端环境设置 SNOWLUMA_ACCESS_TOKEN，宿主从自身环境解析引用。不要将真实值放进示例、命令历史或管理 JSON。保持宿主前台运行：

```sh
"$P4_DEMO/bin/twintail-host" --data-dir "$P4_DEMO/data" --install-dir "$P4_DEMO/packages"
```

第二个终端使用同一个绝对 P4_DEMO 路径。依次 install，再查询操作终态；install 完成后 enable，再查询操作终态，最后查询连接状态。install 与 enable 各自返回 operation_id，替换下列占位符：

```sh
"$P4_DEMO/bin/twintail-host" plugins install "$P4_DEMO/source/adapter-snowluma" --data-dir "$P4_DEMO/data"
"$P4_DEMO/bin/twintail-host" operations get INSTALL_OPERATION_ID --data-dir "$P4_DEMO/data"
"$P4_DEMO/bin/twintail-host" plugins enable adapter-snowluma 0.1.0 '["bots.register","events.publish","services.provide"]' --data-dir "$P4_DEMO/data"
"$P4_DEMO/bin/twintail-host" operations get ENABLE_OPERATION_ID --data-dir "$P4_DEMO/data"
"$P4_DEMO/bin/twintail-host" config get adapter-snowluma --data-dir "$P4_DEMO/data"
"$P4_DEMO/bin/twintail-host" plugins call adapter-snowluma connection.status '{}' --data-dir "$P4_DEMO/data"
```

running 不是完成，必要时重复查询 operation；失败先按错误类别排查，不自动重提业务动作。连接断开不会令 IPC 插件自动退出，可能出现 enable 成功而 connected=false。运行后配置采用 stop→config update→start，每步等待完成，参阅[配置](configuration.md)。

Windows 在新目录构建时复制 `twintail-host.exe` 和 `adapter-snowluma.exe`，安装前把来源清单 entrypoint 改为 `adapter-snowluma.exe`，再运行 install；清单和摘要生成后不能原地改入口。数据根使用当前用户私有 ACL，命令以 PowerShell `&` 调用并按当前版本规则传单个 JSON 参数。Windows P4 运行验证单独记录。

## SDK 集成与事件

适配器申请并获授 `bots.register`、`events.publish`、`services.provide`，分别注册配置账号、发布来源事件和提供服务。清单声明不等于授权，宿主仍检查进程身份与 generation。后台连接任务须保留所有权，stop 取消并 join；stdout 只用于 SDK IPC，诊断走受控 stderr，不能输出秘密或原始完整敏感消息。tungstenite 0.28 的 TRACE 可能打印含 Bearer 的原始握手；当前包为 log 启用 max_level_info 与 release_max_level_info，编译时移除 TRACE/debug，另有捕获日志回归。该特性影响进程内所有使用 log 的依赖，当前 main 不安装 logger；不得为排障直接恢复依赖 TRACE。

OneBot 四类事件分别以 message、notice、request、meta_event 发布，协议名为 onebot.v11。payload 保留原始 JSON，未知字段、未知消息段与 CQ 字符串不能在规范化时丢弃或静默改成文本段。订阅者根据字段识别实际类型，post_type 限1–96字节 ASCII 字母数字与 `._:-/`。未知事件按原始 post_type 作为 event_type 发布，订阅该精确名称即可接收，不映射成统一 unknown，也没有通配订阅。类型库 `account_id` 接受1–128位非负十进制字符串或整数JSON字面量，字符串前导0规范化；负号、小数和指数形式拒绝，原始 payload 不改写。

`Frame` 先按 post_type 存在识别事件，否则要求完整 status/retcode/data 响应结构，而不是“有 echo 就是响应”。事件也可能含未知 echo 扩展；响应须具有对应 status/retcode/data 结构。错误帧和身份不匹配不能伪造另一账号事件。WS ping/pong 只说明连接控制帧交互，OneBot heartbeat/lifecycle 为业务协议事件，不可混作登录或可发消息证明。

宿主事件发布报告 delivered/dropped 是队列准入结果；不表示业务插件已处理，也不证明消息送达 QQ。适配器事件队列、宿主独立订阅队列与 SDK handler 都有限额，溢出产生事件缺口。重连和热重载没有持久事件补发；需要业务去重时保留来源 message_id 与业务键，不能只靠框架 event_id 宣称 exactly-once。

## 动作与服务

业务插件经 `Context::call_action(ActionCall)` 调用 BotRef。OneBot action 名称与 params 传到同一 WS 连接，适配器生成本连接唯一 echo 并维护有界 pending。Echo 在重连后不能复用旧 pending，迟到或未知响应不能满足另一调用。

准入前离线返回 offline，参数非法返回 invalid_request，局部配额/队列满返回 overloaded；这些尚未交给连接发送。成功入队后超时或断连统一保守返回 unknown_outcome，即使可能尚未实际发送，也不自动重试。取消后的业务结果也不能据此认定未执行。远端明确返回的 failed/retcode 与传输不确定不同：保留 status、retcode、data 和原始扩展，业务据返回值判定。status async 只证明远端接受异步处理，不证明最终动作完成。

适配器提供 `onebot.v11@1.0.0` 服务，唯一方法为 `connection.status`，通过显式 provider、service 与 SemVer 范围调用。服务不提供通用动作透传，动作走 SDK action 路由。自定义 SDK request `connection.status` 返回同样诊断，可通过管理 `plugins call adapter-snowluma connection.status '{}'` 查询。响应包含 connected、phase（connecting/connected/backoff/stopped）、generation、connection_attempts、pending、dropped_events、rejected_frames、heartbeats 和 last_error。last_error 为固定诊断类别或 null，不含URL、账号、Token、payload及远端原文。dropped_events 为本地发布队列损失计数，不能代表宿主和业务消费者的全部事件缺口。IPC active、WS connected、账号可用与一次动作执行成功是四个不同证据。

标准 OneBot 发送接口例如 send_private_msg、send_group_msg 或 send_msg；当前 echo 示例使用标准 `send_msg`，按入站消息类型携带私聊 user_id 或群聊 group_id。SnowLuma 部署 echo 时，将独立打包目录中的清单替换为 `examples/echo-plugin/plugin-snowluma.toml`，复制时将目标命名为 `plugin.toml`；该专用清单依赖 adapter-snowluma。原 `examples/echo-plugin/plugin.toml` 强依赖 mock-adapter，不能直接作为真实适配器业务包清单。可执行文件仍使用构建的 echo-plugin，Windows 对该包也须同步设置 `.exe` 入口。对特定 SnowLuma 扩展保留命名和探测，不将其作为其他 OneBot 实现的必需能力。

## 配置、停止与验证

配置沿用 P3 desired/applied 与 prepare/commit 契约。prepare 只验证，不建连接；需要重连的变更可以明确拒绝在线更新，由管理员 stop→离线 update→start 或受控 exclusive 替换。连接关闭须清理所有 pending，不将未完成动作带入新代。插件停止、重载后旧 BotRef 失效；默认先停用活动强依赖消费者，恢复新实例后重新取得 BotRef。

所有队列、帧、并发和等待均有界。队列上限、帧上限和退避范围应以配置 Schema 与实现硬上限双重校验，不能让任意配置恢复为无限资源。SDK 只管理自身回调任务，自建 WS reader/writer/reconnect 任务由适配器 stop 负责取消、关闭和 join。

无凭据验证先使用四类事件和响应脱敏夹具，再使用 mock WS 注入鉴权拒绝、超大帧、未知 echo、断线和动作期限。独立插件进程测试覆盖宿主路由与生命周期。真实部署另按[SnowLuma 验证步骤及矩阵](../compatibility/snowluma.md)记录具体版本、平台与证据；mock 成功不填写真实兼容通过。

## 手动真实服务探针

[real_probe.rs](../../adapters/adapter-snowluma/examples/real_probe.rs) 用于独立核对外部 OneBot 服务；它直接使用 Transport，不经宿主 IPC，不能代替宿主→业务插件闭环。设置私有环境变量 `TWINTAIL_ONEBOT_URL` 与 `TWINTAIL_ONEBOT_SELF_ID` 后运行：

```sh
cargo run --locked -p adapter-snowluma --example real_probe
```

默认只查询 get_version_info/get_login_info，确认 self_id 匹配后计数四类事件60秒，不发送消息。需要 Bearer 时，TWINTAIL_ONEBOT_TOKEN_ENV 指定另一个持有 token 的环境变量名；不通过命令行或 URL 传 token。只有显式 `TWINTAIL_SEND_PROBE=1` 且设置 `TWINTAIL_ONEBOT_TARGET` 才发起一次固定文本的 send_private_msg，不重试，包括结果未知的情况。

等待参数为 TWINTAIL_PROBE_CONNECT_TIMEOUT_SECS（默认10，1–60）、TWINTAIL_PROBE_EVENT_TIMEOUT_SECS（默认60，0–300）、TWINTAIL_PROBE_ACTION_TIMEOUT_MS（默认3000，10–30000）。探针仅输出版本白名单、身份匹配布尔值、status/retcode与计数，不打印账号、端点、token、payload或data；结束显式停止并等待传输任务。该工具不在自动测试中连接真实服务；lead 已执行取得部分真实证据，具体范围记录在兼容矩阵。

## 手动宿主与业务插件闭环

[real_flow.rs](../../adapters/adapter-snowluma/examples/real_flow.rs) 使用 Supervisor 启动独立适配器与临时 feature 插件，经宿主投递真实事件并路由动作，区别于 real_probe 直接调用 Transport。先分别构建适配器与工具：

```sh
cargo build --locked -p adapter-snowluma --bin adapter-snowluma --example real_flow
```

私有终端环境须设置 `TWINTAIL_REAL_FLOW=1`、`TWINTAIL_ADAPTER_BIN`（实际适配器可执行文件的绝对路径）、`URL`（ws/wss）、`SELF_ID` 与 `TARGET`。这里的 URL/SELF_ID/TARGET 是此工具专用名称，与 real_probe 的变量名不同。SELF_ID/TARGET 为无前导零的十进制字符串且必须不同；不要将真实账号写进仓库。可选 `TOKEN_ENV` 指定另一个保存 Bearer 的环境变量名，避免在命令行传明文。`TIMEOUT` 默认180秒，允许1–300秒。变量设置后运行：

```sh
cargo run --locked -p adapter-snowluma --example real_flow
```

只有白名单 TARGET 向配置账号发送私聊、完整文本精确等于 `TwinTail P4 ping`，feature 才尝试一次 `send_private_msg` 回复 `TwinTail P4 pong`。群聊、其他账号、子串匹配及重复匹配均不触发；无论成功、失败或 unknown_outcome 均不自动重试。工具不输出消息正文、账号、端点或凭据，只报告连接诊断、匹配布尔值及结果状态/retcode；子进程不继承完整宿主环境。`--plugin` 是内部子进程模式，不作为人工启动步骤。

该工具需要明确的受控测试会话和一次回复授权，默认自动测试不访问真实服务。工具返回 ok/retcode0 只证明远端动作响应，收件端实际收到应另行确认；真实执行证据按兼容矩阵逐项填写，工具存在不等同闭环已验收。

本次 real_flow 已实际通过：generation1 online→用户断开转发 offline→attempts2…6→generation2 online→matched=true、status=ok/retcode=0、exit=0；用户确认恢复转发后触发并收到一次回复。此结论限定本次 Linux x64 WSL mirrored 客户端与 SnowLuma 2.2.1-node 部署，未记录真实账号与消息 payload。Windows 原生握手诊断不等同 Windows 客户端运行验收。
