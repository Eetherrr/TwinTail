# SnowLuma / OneBot v11 验证记录

P4 已完成 Linux 本地验收及指定真实部署范围验证。客户端为 **Linux x64、WSL mirrored 模式**，经本机转发 `ws://127.0.0.1:9999/` 无 token 连接实测返回 **SnowLuma 2.2.1-node** 的服务。远端服务操作系统、运行提交与发行包 SHA 未确认；Windows 上的 ServerBox 仅提供 SSH 隧道转发远端 SnowLuma，不能据此推定服务进程 OS。已验证直接 Transport 查询/私聊，以及 Supervisor→独立 adapter→临时 feature 在真实断线重连后的单次私聊收发闭环，并取得用户人工收件确认。公开元数据、mock、编译与真实运行分别记录，未测能力见下表。

## 公开资料核对：2026-10-29

| 来源 | 本次查询结果 | 证据范围 |
| --- | --- | --- |
| [仓库 API](https://api.github.com/repos/SnowLuma/SnowLuma) | 默认分支 main | 公开元数据 |
| [main 最新提交](https://github.com/SnowLuma/SnowLuma/commit/8ffd314260e9e2b6967eeca494d87aa5be046c34) | `8ffd314260e9e2b6967eeca494d87aa5be046c34`，2026-10-27，chore(release): v2.2.1 | 查询当时 HEAD，未来可能变化 |
| [最新发布 v2.2.1](https://github.com/SnowLuma/SnowLuma/releases/tag/v2.2.1) | 2026-10-27T09:35:00Z 发布 | 候选发布版本，未下载运行 |
| [固定提交 README](https://github.com/SnowLuma/SnowLuma/blob/8ffd314260e9e2b6967eeca494d87aa5be046c34/README.md) | 声明 OneBot v11、WS 服务端/客户端、HTTP、Windows x64/Linux x64/arm64 | 上游声明，非本机产品验证 |
| [OneBot 正向 WS](https://github.com/botuniverse/onebot-11/blob/master/communication/ws.md) | `/` 在单连接提供动作与事件；`/api`、`/event` 为拆分接口；echo 原样返回 | 标准契约，不能推断全部产品实现 |
| [OneBot 鉴权](https://github.com/botuniverse/onebot-11/blob/master/communication/authorization.md) | 正向 WS 使用 Authorization Bearer；规范也允许 query，本适配器只允许header | 范围收窄，减少URL秘密暴露 |
| [OneBot 事件](https://github.com/botuniverse/onebot-11/blob/master/event/README.md) | 四类 post_type；消息支持字符串或段数组 | 原始形状保留依据 |
| [OneBot API](https://github.com/botuniverse/onebot-11/blob/master/api/README.md) | status/retcode/data；async 表示异步响应，不能据此证明最终业务完成 | 调用结果解释依据 |

以上仅为公开元数据和正文的只读查询记录，非发行包验证。main HEAD 与 release tag 分别记录，未证明二者对应同一提交；正式验收应重新核对发行包、tag解析提交和制品摘要。

## 真实探针记录（2026-10-29）

lead 执行 [real_probe](../../adapters/adapter-snowluma/examples/real_probe.rs)，经用户在 Windows ServerBox 配置的 SSH 隧道转发远端 SnowLuma，本机入口为 `ws://127.0.0.1:9999/`、无 token。初次 WSL mirrored 路径 TCP 可达但 HTTP Upgrade 直接 EOF、无 HTTP 状态，探针等待10秒超时后 stop，未执行查询或发送；Windows 原生复测也读到0字节。用户修复 ServerBox 转发后重试成功，脱敏结果：

- connected=true；get_version_info status=ok、retcode=0，application=SnowLuma、version=2.2.1-node。
- get_login_info status=ok、retcode=0，login_self_id_matches=true。
- send_private_msg **恰好一次**，status=ok、retcode=0；没有自动重试。该证据为远端动作成功响应，用户另已确认收到这条单次测试消息。
- 观察到 message=2、meta_event=3，notice=0、request=0；stopped=true。

上述 real_probe 是直接 Transport 路径；另有以下经过宿主和业务插件的独立证据。

### 真实重连后的宿主业务闭环

lead review 并执行 [real_flow](../../adapters/adapter-snowluma/examples/real_flow.rs)，通过 Supervisor 启动独立 adapter 与临时 feature。仅接受授权白名单私聊的精确完整测试文本，最多尝试一次回复，不自动重试。脱敏执行摘要：

1. connection generation=1、online；用户断开 ServerBox 转发后 offline。
2. connection_attempts=2…6；用户恢复转发后 connection generation=2、online。
3. 用户触发授权私聊；feature matched=true，send_private_msg 返回 status=ok、retcode=0，进程 exit=0。
4. 用户人工确认“已恢复转发并发送，收到pong”。

该证据证明真实重连后 message→host→feature→send_private_msg 单次闭环可用，未保存账号和消息 payload。连接 generation 为适配器外部 WS 代次，不是 Supervisor 插件进程代次。真实 notice/request、群聊、其他动作、Bearer/TLS、旧 echo/结果未知与部署 OS 矩阵仍待测；相关错误和资源边界的本地 mock 结果见下表。运行提交与发行包摘要未知，不把公开 main HEAD 或 release tag 映射为实际运行提交。

## 真实版本矩阵

日期：2026-10-29。下表平台列均指 **TwinTail 客户端**，不映射远端 SnowLuma 服务 OS。客户端 Linux x64 在 WSL mirrored 模式下运行；远端报告2.2.1-node，OS/commit/SHA未知。Windows 仅有全目标严格 Clippy 交叉检查及原生握手失败诊断，无真实适配器运行验收；arm64未测。

| 能力 | 实测传输/范围 | Linux x64（WSL） | Windows x64客户端 | Linux arm64客户端 | 证据 |
| --- | --- | --- | --- | --- | --- |
| 版本/登录查询 | SnowLuma 2.2.1-node | 通过 | 待测 | 待测 | get_version_info/login_info均ok0，账号匹配 |
| 正向 Universal WS | ws，无token，本机转发路径 / | 通过 | 待测 | 待测 | connected=true、正常stop |
| 事件 | message、meta_event | 已观察 | 待测 | 待测 | 探针计数message2/meta_event3；业务message触发闭环 |
| 私聊动作 | send_private_msg | 通过 | 待测 | 待测 | 探针一次ok0且用户确认；业务单次回复ok0且用户确认 |
| 断线重连后业务闭环 | Supervisor+独立adapter+feature | 通过 | 待测 | 待测 | generation1→offline→attempts2…6→generation2→matchedtrue/ok0，exit0 |
| notice/request、未知扩展/CQ | 真实事件形状 | 待测 | 待测 | 待测 | 不能由mock/消息计数替代 |
| 群聊、其他动作、失败/异步响应 | 真实远端语义 | 待测 | 待测 | 待测 | 不从私聊成功外推 |
| Bearer/TLS | 真实鉴权与证书 | 待测 | 待测 | 待测 | 本次无token、ws |
| 旧echo/超时/未知结果、背压/超大帧 | 真实故障边界 | 待测 | 待测 | 待测 | 已有本地mock，不视作真实验证 |

## mock 与夹具证据

| 验证层 | 对象 | 当前记录 | 能证明什么 |
| --- | --- | --- | --- |
| JSON 类型/脱敏夹具 | 四类事件、未知扩展、CQ、响应分类 | 类型库负责人报告9项测试、strict clippy/rustdoc通过 | 解析与保真 |
| mock WS | 鉴权、echo、断线、限额、超时、心跳/控制帧、停止 | 适配器负责人报告9项transport测试通过，含TRACE logger秘密捕获回归 | 本地连接与错误契约 |
| 独立插件进程 | 宿主路由→adapter→mock WS | 适配器负责人报告2项通过：断线重连收发、exclusive reload旧连接关闭后恢复 | IPC 与插件生命周期闭环 |
| Linux loopback CLI | 私有预置→安装启用→desired/applied 1→connected→echo send_msg | 管理测试负责人报告 opt-in cli_p4 smoke 1/1通过（18.91秒） | 首次引导和完整CLI mock闭环，不是外部产品兼容 |
| 真实 SnowLuma | 返回2.2.1-node；远端OS/commit/SHA未知 | 直接Transport及真实重连后Supervisor/feature单次私聊闭环通过，用户确认收件 | 仅限本次指定部署与矩阵已测项，不构成完整兼容认证 |

以上为2026-10-29实现负责人交付结果，文档负责人已核对测试源码，未独立重跑。测试入口为 [transport.rs](../../adapters/adapter-snowluma/tests/transport.rs)、[p4.rs](../../adapters/adapter-snowluma/tests/p4.rs) 与类型库 tests；适配器另报告 strict all-target clippy 和 rustdoc 通过。子进程测试通过 Supervisor 显式注入初始化配置；另有独立 CLI smoke 覆盖首次引导，两者证据分开记录。

## 真实验证步骤与记录模板

1. 从上游发布页选择明确平台制品，记录文件名、SHA256、tag 和提交；按该版本上游部署说明独立启动。记录 OS/架构、QQ/SnowLuma 运行版本及 TwinTail 提交。源码工作树无提交时记录精确源码归档摘要，不能仅写 0.1.0。
2. 在 SnowLuma 配置正向 Universal WS 服务和鉴权，记录脱敏地址、实际路径与 ws/wss 模式。地址及端口来自被测部署，不套用规范中的示例端口。账号和 token 留在私有环境，不写进夹具、提交、截图或操作日志。
3. 按[适配器指南](../plugin-api/adapters.md)构建独立包，安装、离线配置并启用。使用环境秘密引用注入 Bearer。查询 desired/applied 和 connection.status，分别证明配置确认、IPC active 与 WS connected；任一状态不替代真实业务响应。
4. 在受控测试会话发送唯一文本，观察 message 原始字段经订阅到达业务插件；由业务插件调用明确 OneBot 发送动作，记录返回 status/retcode/data 和另一端实际收到的唯一消息。echo 使用标准 send_msg；真实包须使用 SnowLuma 专用依赖清单，参阅适配器指南。
5. 分别采样 notice/request/meta_event 和含未知扩展/CQ 的事件，脱敏后保留原始形状及映射结果。未能触发的事件标为未测，不能补成通过。
6. 中断连接后恢复，确认 connection generation 改变，旧 echo 不完成新调用；在途动作超时/断开应报告结果未知，检查没有自动重发。另测鉴权失败、远端 retcode 失败、超大帧与队列背压。对可能已发出的动作先核对实际结果，不用再次发送来“验证”。
7. 停用业务依赖者后 exclusive 替换适配器，确认旧 BotRef 失效、旧连接关闭和新连接可用。记录事件缺口和 drain_error；不宣称零停机或数据回滚。
8. 记录日期、操作命令、预期/实际结果、脱敏日志或 CI 链接、各矩阵行结论和限制。公开资料、mock、真实运行三个证据来源分别署明。

表格中的所有待测空项在取得以上证据前保留。当前实现范围见 [ADR 0005](../architecture/0005-p4-onebot-snowluma.md)，既有平台验收见[总记录](README.md)。
