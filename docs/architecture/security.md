# 安全边界

P4 已完成 Linux 本地验收及指定真实部署范围验证；P3 及 P1/P2 Linux 本地验收已通过；本页描述实际控制和限制。P0 历史见 [ADR 0001](0001-p0-baseline.md)，当前架构见 [ADR 0004](0004-p3-management-config-reload.md) 和[插件 API](../plugin-api/README.md)。

## 信任与权限

仅运行操作者信任的本地插件。**独立进程与框架权限不是操作系统沙箱**：插件仍拥有宿主用户的文件、网络和进程权限，同用户代码也可能直接访问本机管理端点。宿主能力仅约束经过宿主协议的方法，不隔离恶意本地代码。

清单申请与管理员 grants 取精确交集、默认拒绝，无通配符；必需能力未授权时拒绝加载。身份绑定到宿主创建的管道，hello 不可自提权限。P2 提供 bots.register、事件订阅/发布、动作和服务路由。路由绑定源实例身份及代次，动作按 BotRef 定位目标，服务显式选择 provider 与 SemVer。动作/服务授权仍为粗粒度能力；未按动作名、账号、服务方法或参数细分。存储和插件管理能力词汇仍无对应插件RPC。

## 包与进程

校验合法 id、SemVer/API、相对路径、真实包/入口边界及强依赖。拒绝链接、特殊文件、目录穿越、重复身份与超限包。本地目录包使用同根 staging 原子发布、不可覆盖版本和 OS 写锁；SHA256 清单用于损坏检测。包必须在校验至执行期间保持不可变，仍有同用户替换竞争；没有签名、来源认证或归档解包支持。

正常运行创建私有数据根；--check 不读写，--validate 读取配置和包但不执行。包与 DATA/plugins/id/data 分离，私有路径命名不构成文件系统沙箱。插件环境默认清空，仅转发 PATH/SystemRoot/WINDIR/TEMP/TMP，P3 将配置中的环境秘密引用在宿主解析后注入配置快照，不把完整宿主环境交给插件；这不限制同用户插件自行访问文件和进程。

P1 自动重启预算 0；failed 需显式 start，真正启动新进程时 generation 增长。只清理和等待直接子进程，**禁止插件派生守护子进程**；没有进程树、Windows job object 或通用后代清理保证。

## IPC 与日志

插件单帧上限 8 MiB，先验长度后分配；control/response 独立队列 8/32、入站及命令队列各 32、宿主在途请求 32、插件每连接累计 request ID 4096。默认握手10秒、请求5秒、停止2秒，写入期限2秒。非法关联、超时或连接故障使代次退役，取消后的部分帧连接不复用。队列限额不是完整内存/CPU 沙箱，详见[协议](../plugin-api/protocol.md)。

stdout 专用于帧；stderr 以固定4096字节缓冲持续读取并丢弃，仅约束缓冲占用，没有完整日志限速、CPU限制、查询或脱敏展示。错误消息不得含秘密、完整敏感输入或原始插件输出。P3 管理读保留秘密引用，操作记录仅安全摘要；没有秘密保险库、自动轮换或浏览器配置系统。

## 本机管理

Unix DATA 由有效用户拥有且精确0700，management.sock为同用户socket且0600；双方检查peer UID。校验祖先所有者、可写权限及链接，拒绝不安全根；已有目录不自动chmod，既有socket不自动删除，崩溃残留须确认无宿主后手动清理。退出仅清理由当前服务创建且身份未变的端点。

管理与插件管道独立，1 MiB帧、最多16连接、每连接一次请求、网络读写默认5秒；操作等待期限以代码为准，超时后先查询状态。Windows受限本机管理已实现并通过交叉编译/Clippy，运行待Windows CI，不据Unix结果宣称Windows访问控制通过。

## 后续边界

P3 提供离线 Schema 校验、配置 CAS/秘密引用和本地包/重载管理；禁止外部 Schema 引用，配置/锁使用私有权限和链接校验。P4 SnowLuma 正向 Universal WS 适配器已实现，P5 Web管理已完成Linux本地验收，采用下述认证、鉴权、CSRF/Origin和CSP边界，仅允许loopback。SnowLuma独立部署，真实QQ及具体版本验收结果以[兼容矩阵](../compatibility/snowluma.md)为准。

Windows 管理命名管道为 token user SID 配置 protected DACL，拒绝远程连接，并双向核验对端进程 token SID；首次创建拒绝命名冲突。四项 Windows 专用测试已编译但尚未在 Windows 运行，跨用户拒绝也未实测。该控制不隔离同用户恶意插件。ACK/EOF 传输和取消关闭用于限制管道 flush 任务的生命周期。

P2 控制、响应和事件分队列；事件投递溢出只影响对应订阅者并返回 dropped 计数。注册、队列和在途转发有界，但内存上限不是完整进程资源隔离。已入队动作断连/超时为 unknown_outcome，禁止自动重试。SDK 回调必须协作式让出执行权，插件自己创建的任务仍由插件取消/回收。详见 [ADR 0003](0003-p2-sdk-routing.md)。

## P4 外部连接边界

adapter-snowluma 只连接配置的单个 self_id 和正向 Universal ws/wss，拒绝URL userinfo/query/fragment；Bearer 通过宿主解析环境引用后仅注入 Authorization。wss 校验服务端证书。配置不导出 Debug/Serialize，状态只返回固定类别与计数，不含端点、凭据、payload或远端错误原文。tungstenite TRACE 会打印原始握手，包使用 log 的 max_level_info/release_max_level_info 编译时移除TRACE/debug，且有秘密捕获回归；该设置影响进程内所有log依赖，不能为排障恢复依赖TRACE。

帧、pending、事件队列及期限均有硬范围。账号不匹配、未知/迟到echo不能完成其他调用；重连不重放已准入动作，超时或断线保守返回 unknown_outcome。事件队列溢出有缺口，WS ping/pong与OneBot heartbeat分开。外部WS失败退避至配置上限后持续尝试直到stop，无jitter和总次数预算；宿主进程自动重启预算仍为0。真实QQ不进入无凭据CI，日志与夹具不得泄漏秘密。具体资源默认与限制见[适配器指南](../plugin-api/adapters.md)。

## P5 Web 管理边界（Linux本地验收通过）

web-admin 作为独立受信任插件桥接宿主管理 IPC，默认只监听 `127.0.0.1:17280`；P5不支持任意网卡远程暴露。凭据由宿主环境秘密引用解析，token至少32字节，不写URL、浏览器存储或普通日志。登录使用HttpOnly、SameSite=Strict会话cookie，管理调用另须CSRF与Origin校验。慢请求体在初次认证后仍可能跨越会话期限，管理操作准入前须复查会话；当前会话固定默认3600秒，默认32会话/16执行请求/64TCP连接、64KiB body/10秒请求期限，登录body至多1KiB。严格匹配Host/Origin、拒绝query和跨站来源，静态响应配置CSP与no-store；无远程代理/TLS部署模式。具体上限见Web指南；Linux HTTP/CLI与真实Chromium桌面/手机验收通过，Windows运行、远程部署及人工视觉审查不在该结论范围。

POST manage 桥接按具体管理操作要求 exact `management.read` 或 exact `management.write`；write不隐含read。浏览器认证不替代宿主权限，未知管理方法不能进入任意转发通道；桥接不提供 plugins.call。adapter.status 只返回连接状态有限字段。config.get/schema 通过 ConfigStore 的 read_for_management / validate_management_input 实施 Schema writeOnly 边界：秘密字段仅接受环境引用，不返回resolved值。admin_token/access_token 标记 writeOnly；旧字面量秘密在Web路径安全拒绝，需CLI迁移，不返回可被写回的占位符。组合Schema秘密约束保守合并。管理日志仅限最多128条的有界脱敏操作journal投影，不包括插件stderr、消息正文或凭据。安装、配置和重载沿用宿主校验、CAS与操作状态，不由浏览器直接写文件。

端口占用、配置拒绝、自停和重载失败保留独立本机CLI恢复入口。关闭页面不会回滚已接纳操作；页面断开后先查询operation。Web自身停止/排他替换可导致HTTP断连，不保证零停机。完整操作和恢复路径见[Web管理指南](../plugin-api/web-admin.md)，测试结果在[兼容记录](../compatibility/README.md)取得证据后填写。

## P6候选与反向连接边界

当前候选只在本地构建验证，不外发、不执行public publish。项目无license/license-file与许可正文，权利人许可决策是外发gate；摘要/第三方依赖许可汇总不代替项目授权。相关策略见[安全与依赖升级](../release/security-updates.md)。

reverse_ws只绑定数字loopback，默认127.0.0.1:17281，非loopback即使配置token也拒绝。精确路径默认/onebot/v11，不接受query；要求精确单X-Self-ID匹配配置账号与单X-Client-Role: Universal。可选token存在时只接受精确单Authorization: Bearer TOKEN，重复头不能绕过认证。仅一条活动socket，额外连接拒绝，通常返回HTTP409；409为best-effort try_write后drop，避免恶意附加peer阻塞主连接，不保证所有网络下客户端都收到HTTP响应。token不匹配401，账号/role/path不匹配403；bind冲突InvalidConfig，stop需join释放监听。重连连接generation增长，在途断线/超时UnknownOutcome且不重放；错误不回显远端正文/token。无native TLS、split或HTTP，远端接入只通过外部SSH隧道映射loopback。

上述按P6实现合同核对；负责人报告反向mock、真实SDK子进程闭环及同端口reload重绑定通过，最终全工作区源码回归通过，不扩大已有真实SnowLuma结论。发行/状态/合同与soak工具及平台执行范围见[候选发行](../release/README.md)，备份可能含敏感业务数据，不能把摘要当作加密或签名。
