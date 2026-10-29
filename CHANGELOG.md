# Changelog

所有项目改动均在此记录；尚未发布的改动归入 Unreleased。采用 Added、Changed、Fixed、Removed、Security 分类，发布条目使用版本及日期。规划功能不代表已实现。

## [Unreleased]

## [0.1.0] - 2026-10-29

### 发行准备

- 公开仓库 https://github.com/Eetherrr/TwinTail 以 main 为默认分支；原生平台 CI [40901049665](https://github.com/Eetherrr/TwinTail/actions/runs/40901049665)（ad3296b09790e3ea497aac39b41957b66aabf3cd）四任务全部通过：Linux x64/arm64 各184项Rust（含显式CLI smoke）、release19/state16；Windows MSVC157项Rust、release19及独立文档/空宿主/生成合同检查；Web24项单元、4项模拟浏览器、类型/构建/资源漂移检查。两项真实宿主浏览器测试在CI按配置跳过，原本地验收记录保留。v0.1.0 附件仅提供 Linux x86_64，实际公开状态及摘要由 GitHub Release 记录。

- 经项目方授权采用 MIT，新增 LICENSE、workspace/全部 Rust 包与前端 license 元数据，以及第三方依赖许可正文与再分发声明；GitHub 新建公开仓库，以 main 分支发行。
- 发行前审查待提交库存、秘密与隐私数据、生成物及 .gitignore，补充嵌套构建目录、环境文件、运行日志/socket/PID、编辑器和 Python 缓存排除规则；保留 Cargo.lock、前端锁文件、协议 Schema 和正式内嵌 Web 资源。
- 更新发行工具与文档，移除许可证未决标记，重新生成带 MIT 和第三方许可正文的发行归档。目标平台/真实 SnowLuma reverse 验证范围保持原证据边界。
- 第三方许可清单覆盖 Cargo.lock 全部 206 个第三方版本及 Web 25 个生产安装路径；补齐 12 项离线缓存缺口，10 个 Windows 精确版本归档均核对锁文件 SHA256，uuid-simd/vsimd MIT 原文按发布包 VCS commit 固定来源取回。所有清单条目均含正文，保留 Unicode、ring、TypeScript 等额外声明。
- MIT 元数据 9/9 workspace packages、发行工具 19 tests、格式与忽略规则检查通过；首次公开候选扫描未发现真实秘密或 QQ 数据，合成 fixture 保留；正式 Git 提交使用 GitHub noreply 邮箱。

### Fixed

- 首次 GitHub Linux x64/arm64 CI 暴露停止握手测试的同步竞态：mock 先确认完整 HTTP Upgrade 请求再取消，保留原 250ms 取消/EOF 门槛和 Drop 关闭检查。
- 修复原生 Windows 构建 Web 内嵌资源时误拒绝目录分隔符；按安全 UTF-8 路径组件生成 URL，拒绝绝对路径、parent、非法前缀等，并增加 Linux/Windows 原生路径回归测试。

- 修复 Windows 配置路径检查提前探测裸盘符/verbatim prefix 的问题，拒绝盘符相对路径及 junction/reparse point，增加普通/extended 路径加载、原子配置恢复和 junction 回归。
- 将反向重复连接测试与鉴权测试分离：鉴权仍严格要求 401/403；best-effort 重复连接仅接受 409 或特定 ConnectionReset，并验证原认证连接的动作回环和代次所有权。
- Web CI 的 Chromium 安装在下载 100% 后无输出停留十二分钟；改用与锁定 Playwright 1.56.1 匹配的官方预装浏览器镜像，保留全部浏览器交互/生产构建/资源漂移检查并设置任务期限。容器中的资源漂移检查显式使用 workspace 和单次 safe.directory，兼容 checkout 与后续步骤的用户/工作目录差异；workspace 测试使用 --no-fail-fast 收集同轮全部失败，失败仍使任务不通过。

- 修复 Windows 包校验在转换原生路径分隔符之前调用 wire 路径规则的问题；仅内部枚举路径按安全组件转换，外来 manifest/receipt 和 Unix 字面反斜杠仍严格拒绝，增加三项回归。
- 启动查询回归改用 fixture 的 held-handshake marker/release 屏障验证管理请求在启动阻塞时仍返回 running，再验证重启中断恢复；保留有界读取，消除依赖共享 runner 调度速度的 500ms 断言。

- 固定项目文本 checkout 使用 LF，消除 Windows core.autocrlf 对 Schema/OpenAPI 精确漂移检查的影响；第三方许可证使用 -text 保留上游原始字节，不通过放宽生成合同比较规避漂移。

- 包安装完成 receipt 同步后显式关闭写句柄，再执行 staging 目录重命名，保留同步、路径校验和原子发布顺序，兼容 Windows 文件句柄约束。

- 包安装回归按 canonical 路径比较预期位置，兼容 Windows 的扩展长度路径前缀；保持 PackageStore 返回规范路径的现有 API。

### P6 Added

- 新增 ADR 0007 与发行、运维、平台、插件合同及迁移指南；保持 wire 1.2、包版本 0.1.0 和 config_version 1，旧正向配置省略 transport 仍兼容。
- SnowLuma 插件新增 reverse_ws：仅数字 loopback 监听，精确路径、Bearer 与唯一账号/Universal 身份头；单活动连接、启动占端口、停止释放和 exclusive 重载，复用动作 echo、连接代次与 UnknownOutcome，不自动重放。
- 新增标准库候选工具：显式 Linux x64/arm64 GNU 与 Windows x64 MSVC 布局，五个二进制、平台清单与依赖文档白名单、SHA256 库存、确定性 tar.gz/zip、严格归档/解包校验、无覆盖与安全错误。候选包含 CLI/Schema/运行文档，构建和契约 Cargo 示例仍需同版本源码。
- 新增 Unix 冷状态 snapshot/verify/restore：持有实际宿主管理端点和配置锁，保留原字节、秘密引用及 desired/applied 差异，恢复到新私有空根，完整性和失败清理检查；不备份安装根、不解析环境、不自动迁移业务数据。
- 新增真实 Supervisor 插件 contract_check 与 bounded_soak，显式受信任包和权限、配置 Schema/秘密解析、可选健康检查、确定性退出码；有限期限/工作量的慢消费者过载和 echo/action 逐条验证，并观察直接子进程清理。
- 新增手动候选构建工作流，定义 Linux x64/arm64、Windows x64 原生构建/测试/归档矩阵；CI 增加 arm64 runner 以及发布、状态工具测试。工作流只保存候选 artifact，不创建 tag 或公开发布；远端执行尚无证据。

### P6 Fixed

- 反向 SDK 子进程测试隔离并发正向连接的端口预约竞争，并将保留端口的释放移至启动前；增加固定端口占用导致启动失败的子进程回归。外部 OS 竞争窗口仍存在，不把测试隔离宣称为原子端口交接。

### P6 Security

- 新增 SECURITY.md、离线恢复与受控依赖升级/回退说明。发布与备份拒绝路径穿越、链接、特殊文件、库存/摘要篡改和覆盖既有目标；SHA256 仅检测完整性，不认证发行者。
- 官方 OSV 实查 206 个 Rust 依赖，报告 block-buffer 0.10.4 的 GHSA-qwgh-2vcv-g2f7 宽范围命中；锁定源码与 advisory 的游标布局不同，独立 panic PoC 未复现，保留原始结果、适用性疑问和验证限制。未盲目升级依赖；npm audit 报告零漏洞。详见依赖审计记录。
- 本地候选不替代公开发行授权；许可证正文和 Cargo license 元数据仍待项目方确定，Windows MSVC/arm64 原生运行、真实 SnowLuma reverse 及无限期运行未获认证。

### P6 验证（2026-10-29）

- Linux 全 workspace 默认 174 项测试及 1 项 doctest 通过，另显式运行 2 项 CLI 回归通过（发行构建宿主，P4 6.39 秒、P5 15.55 秒）。格式、全目标严格 Clippy、严格 rustdoc、协议/清单 Schema 和 OpenAPI 漂移通过；适配器含 19 项测试、SDK 新增 3 项契约/短负载集成测试。
- 冷状态 16 项及发行工具 18 项 Python 测试通过；真实 snapshot/verify/restore 保留 revision9/applied8 与秘密引用，再次恢复到非空目标被拒绝。实际 Python 3.13.5，最低 3.11 等待 CI；Windows 工具恢复不支持。
- Windows x64 GNU 全目标严格 Clippy 交叉检查通过，未声称 MSVC 构建或目标运行。前端 npm ci、24 项 Vitest、类型检查、生产构建和内嵌资源重建一致性通过；P5 浏览器历史验收范围不扩大。
- Linux x64 五个 release 二进制构建完成；本地候选归档与独立验证报告保存在 dist/，最终 SHA256、同输入双 stage/归档、系统 tar 解包及污染/篡改拒绝结果以 release-validation.json 与 SHA256SUMS 为准。确定性针对相同构建输入的归档，不代表不同机器上的编译产物逐字节一致。
- 60 秒健康事件输入的真实 mock/SDK 有界负载通过：6,000 次 send_msg 逐条核对账号与唯一消息序号，24 代/24 健康检查、49 个直接子进程 /proc 确认退出。总 6,128 offered、6,033 delivered、35 dropped、60 个慢实例退休后无订阅者事件，含过载/清理耗时 63.976 秒，历史峰值 256；未量测内部 pending/task 或内存，非真实 SnowLuma 性能认证。

### P5 Added

- 新增独立进程 `web-admin` 插件与 Vue 3 / TypeScript / Vite 管理台，提供插件列表/详情、本地包安装与显式授权、启停/卸载/排他重载、Schema 与 JSON 配置编辑、desired/applied 修订、操作轮询和有界安全诊断。静态资源随插件编译嵌入；Node 26.7.0 / npm 12.2.1 与依赖锁文件固定。
- 新增 `ManagementCall`、SDK `Context::management()`、协议 Schema 与绑定插件身份/instance/generation 的有界管理桥接。精确区分 `management.read` / `management.write`，固定方法白名单，禁止通用 `plugins.call`。宿主复核来源租约、绝对期限与取消状态，mutation 先返回 operation 受理结果，后台执行自身停止/重载。
- 新增 HTTP/OpenAPI 合同、ADR 0006、Web 打包运行与 CLI 恢复指南，以及管理桥接、真实宿主/HTTP 闭环和前端交互测试。CI 增加前端类型检查、测试、浏览器交互、构建与发布资源漂移检查，以及 Linux Web CLI smoke。

### P5 Fixed

- 真实浏览器并发查询发现大包同步 SHA 校验会阻塞单线程宿主并触发生命周期超时；管理磁盘查询/包变更改用有界阻塞任务，View 仅在复制快照时持锁。包列表采用轻量安装清单并明确 `verified:false`，启用/重载保留完整校验；前端库存定时刷新降低至 30 秒。

- 浏览器会话切换时丢弃旧响应；异步 CAS 冲突保留原草稿与修订；只读授权禁用变更操作；定时刷新安全日志不再清除适配器查询错误。

### P5 Security

- Web 默认绑定 `127.0.0.1:17280`，仅允许数字 loopback 地址；精确 Host/Origin、HttpOnly SameSite=Strict Cookie、CSRF、固定会话期限、登录/退出限流及有界连接/并发/请求体/请求时限。慢请求体结束后再次检查会话与 CSRF，登出和到期后不继续准入。
- 管理配置读取保留未解析 `$secret` 引用，Schema `writeOnly:true` 字段拒绝通过 Web 读取或写入明文字面量。旧配置通过 CLI 迁移，不生成可回写脱敏占位符；HTTP 另阻止响应包含当前管理令牌。诊断、操作结果与错误采用安全投影，不透传业务日志或后端原始诊断。
- 配置编辑器保留草稿与基础修订，异步操作及冲突不自动覆盖；读写检查 JavaScript 安全整数范围，避免大整数在浏览器往返时舍入。

### P5 验证（2026-10-29）

- Linux 全量默认执行 163 项测试与 1 项 doctest 通过；另显式运行 P4、P5 两项真实 CLI 测试通过，覆盖首次配置/安装、HTTP 授权、CAS、重载换代、自停/端口冲突后的 CLI 恢复。最终宿主包详情轻量查询另补 P3/P5 回归与严格检查通过。
- 格式、全目标严格 Clippy、严格 rustdoc、协议/清单 Schema 与 OpenAPI 漂移通过；Windows x64 GNU 全目标严格 Clippy 交叉检查通过，Windows 实际运行未验证。
- 前端固定依赖 `npm ci`、24 项 Vitest、类型检查、生产构建及资源重建一致性通过；依赖 audit 为零已报告漏洞。真实 Chromium 桌面/手机 4 项模拟交互通过，真实宿主 2 项浏览器闭环通过（桌面/手机），验证在线不支持时草稿保留、停机后 CAS 更新成功、重载代次递增与 desired/applied 一致、正式适配器安全状态及退出登录。截图已产出；无人工视觉审查。
- 修复后真实四插件并发查询均返回 200，包清单由约 10.8 秒降至约 11 毫秒，插件保持 active；正式适配器安全状态可显示未连接。无 QQ 或外部服务依赖，不扩展 P4 真实兼容范围。
- 详细范围、操作步骤及验证证据见 ADR 0006、Web 管理指南与兼容矩阵；远程管理/TLS、原始日志订阅与 Windows 运行验证仍待后续阶段。


### P4 Added

- 新增独立 `twintail-onebot11` 类型库，支持四类事件与未知扩展、原始消息段/CQ 字符串及任意精度账号/扩展数字，规范化为既有 `PublishEvent`，保留动作响应的 `status`、`retcode`、`data`、未知字段与 opaque echo。
- 新增独立进程 `adapter-snowluma` 插件：正向 Universal `ws/wss`、Authorization Bearer、显式单账号 BotRef、连接代次与 echo 关联、有界帧/事件队列/pending、指数退避重连、独立 ping/pong；请求与 `onebot.v11@1.0.0` 服务提供安全连接状态。
- 适配器采用 exclusive 替换、启动后台连接、停止取消并等待任务；配置变更要求停止后更新再启动。动作准入后超时/断线返回 UnknownOutcome，不自动重放。
- 增加合成脱敏协议夹具、loopback WS 故障测试、真实 Supervisor/SDK 子进程消息闭环和排他重载测试，以及适配器作者指南、ADR 0005 和独立 SnowLuma 兼容记录。反向 WS、split API/event 和 HTTP 留待后续。

### P4 验证进展（2026-10-29）

- Linux 全仓137项测试及1项rustdoc示例通过；额外 opt-in CLI smoke 1项通过，覆盖私有配置首次预置、安装/启用、desired/applied、正式echo依赖清单与消息闭环、正常退出。最终手动工具已通过全目标严格Clippy/编译回归；real_flow另以临时mock验证负样本拒绝及重复触发只发送一次。
- 严格格式、全目标Clippy、rustdoc和既有Schema漂移检查通过；Windows GNU全目标严格Clippy交叉检查通过，未声称Windows运行/远端CI通过。
- 真实端点修复本机转发后，`real_probe` 确认 `SnowLuma 2.2.1-node`、登录账号匹配、授权单次 `send_private_msg` 返回 ok/0，用户确认收到；观察message/meta_event并正常停止。账号、会话、消息正文及token不入库，直接Transport证据不冒充完整宿主业务闭环。
- 真实 `real_flow` 经 Supervisor 与独立 adapter/业务插件验证：用户中断再恢复SSH转发，连接 generation 1→2（共6次连接尝试）后，指定账号精确短语触发唯一一次 `send_private_msg`，返回 ok/0；用户确认收到回复，验证进程退出0。
- P4已完成Linux工程验收及指定真实部署的版本/登录/私聊/重连闭环验证。远端操作系统、发行包摘要/实际提交未知，不将客户端WSL证据扩展为SnowLuma多平台认证；真实notice/request、群发、Bearer/TLS及其他动作仍待独立验证。反向/split/HTTP未实现。
- CI增加host/适配器构建与Linux无凭据CLI smoke；真实QQ手动工具需要显式环境开关，不进入自动CI。新增手动`real_probe`查询及可选单次发送，`real_flow`用于目标白名单/精确短语/单次回复的宿主业务闭环验证。

### P4 Changed

- echo 示例使用 OneBot v11 标准动作 `send_msg`；原 mock 示例接受任意动作名，调用方如断言 `send_message` 需同步调整。新增 `plugin-snowluma.toml` 清单供打包时选择真实适配器依赖，原清单保留 mock 依赖。
- workspace/锁文件纳入 OneBot、WebSocket、TLS 依赖；serde_json arbitrary_precision 在 workspace 特性合并后启用。协议手写反序列化避免 Serde untagged/flatten 中间值丢失大数支持。

### P4 Security

- token 只进入 Authorization，不接受 URL 用户信息/query/fragment；固定诊断避免回显配置、远端正文或 token。适配器启用 `log` 编译级 INFO 上限，去除依赖中可能输出完整鉴权握手的 TRACE/DEBUG；该静态上限影响同一构建中使用 log 的依赖。
- 修复持续事件读取可能饿死动作、取消 stop 后丢失任务所有权、旧连接排队动作跨连接发送及 peer Close 未刷出的审查/测试发现。

### Added

- P3：新增版本化本地包安装/inventory/卸载、SHA256 完整性检测、同版本禁止覆盖与 OS 文件锁；拒绝链接、特殊文件、路径越界及超限包，支持带合法摘要的来源包。
- 新增配置 Schema/CAS 存储、TOML envelope、环境秘密引用、desired/applied sidecar 与跨实例 OS 写锁。Schema 禁止外部引用，配置、锁和 schema 拒绝链接，错误不包含实例值。
- 新增管理 actor、包/配置/操作 CLI、有限持久 operations.json、运行中只读查询、幂等键及 interrupted 恢复标记；安装与启用分离，卸载保留配置和业务数据。
- 新增 exclusive 热重载、排空诊断、先回收旧进程后启动候选、候选失败恢复旧包/最近确认配置、代次隔离和取消续作；parallel_safe 明确返回 Unsupported。
- 新增配置/包/SDK/真实进程回归与 ADR 0004，扩充配置、存储、热重载、迁移和生命周期 API 指南。

- P2：新增 `twintail-plugin-sdk`，封装握手、生命周期、取消、有界事件/RPC 和 typed 路由客户端；新增可打包运行的 `echo-plugin`、`mock-adapter`，通过 mock 事件→echo→mock 动作验证无需 QQ 账号的闭环。
- 新增绑定进程身份/代次的 BotRef、账号注册、精确事件订阅/发布、动作和显式提供者的 SemVer 服务路由；独立事件队列及 `DeliveryReport` 暴露慢订阅者投递溢出。退出撤销注册，禁止跨代次借用旧路由。
- 新增本地同用户认证的 `plugins call ID METHOD JSON` 管理命令；新增九份 P2 载荷 Schema、ADR 0003、SDK quickstart 与事件/动作/服务开发指南，扩充相关回归与故障测试。

- P1 Linux 本地验收通过：新增 `twintail-protocol` wire v1、8 MiB 有界帧与结构化错误；schemars 导出 protocol/hello/initialize/manifest/host-config 五份 Schema，并提供漂移检查入口。
- 新增 `twintail-core` 清单与 SemVer 校验、路径边界、精确权限交集、强依赖拓扑计划、握手与直接子进程监管；有界独立控制/响应队列、超时与故障清理，自动重启预算为 0。
- 新增基础 `plugins list/status/start/stop --data-dir` CLI、离线 `--validate` 和 Unix 本机管理通道；Windows 后端已实现并通过交叉编译/Clippy，运行验证待 Windows CI。新增 Serde/serde_json、schemars、toml、semver、Unix rustix、Windows interprocess 2.4.7/winsafe/widestring/sha2 及测试 tempfile 依赖，并扩充 Tokio 进程/IO/同步/文件系统能力。
- 新增清单/计划/权限、wire 与无 SDK 子进程故障注入测试 fixture；新增插件 API 协议、清单、生命周期、权限、错误与排障文档，提供 fixture 手工打包验证流程。fixture 不是正式插件样例；本轮 Linux 通过 50 项测试及 1 项 rustdoc 示例，最终命令与平台限制见兼容性记录。

- 2026-10-28：建立项目级 `SKILL.md`，完成 Rust/OneBot v11 QQ Bot 框架架构分析，明确最小宿主、独立进程插件、SnowLuma 适配器插件与 Web 管理插件的边界。
- 设计版本化 IPC、双向事件/动作路由、插件清单与能力、依赖管理、配置修订与秘密处理、热重载及失败恢复契约；明确无损投递、外部副作用回滚和进程安全隔离的限制。这些契约尚待实现。
- 制定 P0–P6 开发阶段及验收门槛，规划 Rust SDK、插件 API 指南、示例、Schema/OpenAPI、SnowLuma 兼容验证和发布检查。
- 新增 `AGENT.md`，规定开发流程、架构边界、文档与验证要求，以及所有项目改动同步更新 CHANGELOG 的规则。
- 新增 `AGENTS.md` 作为自动发现入口，引用 `AGENT.md` 与 `SKILL.md`，避免重复维护执行规则。

### Changed

- P3 wire 升级为 1.2，初始化携带可选配置快照，SDK 新增 config.prepare/commit/abort/status 及 lifecycle.started/drain；SDK 保留接受 1.1 及更高 minor，配置/启动确认扩展要求双方支持 1.2，旧 1.0/1.1 插件保留基线握手。显式配置要求应用确认，legacy 无 schema/revision0 保留兼容。
- P3 同步项目阶段、架构与 API 文档、manifest/config Schema 及依赖锁文件；修复包锁遗留、配置 CAS 竞争、prepare 清理、排空响应与旧配置回退等审查发现。

- wire 升级为 1.1，保留 1.0 插件的 P1 信封/握手兼容；P2 新增方法需双方支持，新增 `unknown_outcome` 描述动作入队后结果丢失，不自动重试。清单 `protocol_version` 仍为 major 1；Cargo/host_api 保持开发版 0.1.0。P2 SDK 的握手基线为 1.1，P3 仍保留该兼容门槛。
- 更新根文档、架构与 API 指南以区分已实现的 P2 路由和 P3 配置/热重载、P4 SnowLuma、P5 WebUI；同步 workspace/依赖锁文件与 CI 示例构建范围。

- P0→P1：正常运行由只显示路径改为读取 DATA/host.toml、验证包、创建/检查私有目录并启动已配置插件；配置缺失表示空计划。迁移时为已有 Unix 数据根设置正确用户归属及0700权限，socket要求0600；残留管理socket确认无宿主后手动清理。`--check`保持引导检查且不读写，离线包验证使用`--validate`。
- 同步README、AGENTS 自动发现入口、执行约定、设计和架构/安全/兼容入口为“P1 Linux 本地验收通过”；保留P0历史证据。SDK、路由、配置事务、热重载、SnowLuma与WebUI仍未实现。

- 2026-10-28（P0）：将初始化单包迁移为 resolver 3 的 Cargo workspace，新增 `twintail-host`，使用 Rust 2024；Rust 工具链与当前最低支持版本固定为 1.99.1，提交依赖锁文件。原 `TwinTail` 包与 `src/main.rs` 由宿主 crate 替代，启动入口改为 `cargo run -p twintail-host`。
- 空宿主支持默认等待 Ctrl-C/Unix SIGTERM、`--check`、`--help`、`--version` 和数据/安装目录参数；P0 不创建运行目录或加载配置，非法参数返回退出码 2；支持平台在输出等待信号前完成处理器注册，避免就绪后立即发送信号触发默认终止。
- 建立 Linux/Windows GitHub Actions 检查流程，覆盖格式、Clippy、测试、文档与空宿主启动；增加 CLI 集成检查，忽略本地运行数据、插件包与环境秘密文件。
- 补充 README、P0 ADR、架构索引、安全边界和兼容性记录，同步项目阶段状态。插件加载、SnowLuma 对接、热重载及 WebUI 尚未实现，未进行真实 QQ 联调。

### Fixed

- 启动/停止 future 取消后继续保留任务所有权；shutdown 等待已登记启动及直接子进程清理，管道任务取消时自动 abort。管理启停可响应退出信号，慢握手期间持续协调依赖故障。
- 管理网络阶段保持 5 秒期限，生命周期响应等待独立为 300 秒，超时提示先查询状态；Windows 通过 ACK/EOF 与取消关闭避免未读响应丢失及后台无限 flush。
- 指定宿主 default-run，避免测试 fixture 二进制使 cargo run 歧义；CLI 帮助对齐实际 management.sock，host.read 文档标记保留未实现。

### Security

- Windows 本机管理使用当前 token 用户 SID 专用 protected DACL、远程拒绝、首实例绑定及双向 peer token SID 校验；交叉 Clippy 已通过，Windows 运行及跨用户拒绝仍待目标环境验证。
- Unix 管理拒绝不安全归属/权限/链接及端点替换误删；退出取消并 join 管理 handlers。权限控制不构成同用户插件沙箱。

### P1 验证记录（2026-10-29）

- Linux x86_64，固定 Rust/Cargo 1.99.1：fmt、严格 Clippy、50 项测试与 1 项 rustdoc 示例、警告即错误的文档构建、两项 Schema 导出漂移检查通过。
- 独立 fixture 覆盖启停、版本、坏帧/超大帧、超时、越权、崩溃、依赖级联、取消 start/stop、背压与旧代迟到响应。真实 CLI 完成离线验证、list/status/stop/start、generation 递增及 SIGTERM 退出和 socket 清理。
- Windows GNU target 主工程全目标严格 Clippy 通过，包含四项 Windows 管理测试的编译；未运行 Windows 测试、远端 CI、arm64 或真实 SnowLuma。

### P0 验证记录

- 本机 Linux x86_64 使用已安装的 Rust/Cargo 1.99.1（`+stable`）通过格式、Clippy、3 项 CLI 集成测试、文档构建和空宿主启动检查；含空格路径不产生文件，就绪后立即 SIGINT/SIGTERM 各 5 次均正常退出。
- P0 验证时独立 `1.99.1` rustup 工具链仍在下载，使用同版本 stable；P1 验收时固定工具链已安装。Windows、arm64、远端 Actions 和真实 SnowLuma 兼容性尚未验证。

### P3 验收记录（2026-10-29）

- Linux x86_64、Rust/Cargo1.99.1：格式、全目标严格Clippy、workspace测试（117项+1项rustdoc）、严格rustdoc、17份Schema漂移、空宿主检查全部通过；131个相对文档链接有效。
- 新增10项配置存储、7项包管理、6项宿主重载进程、6项SDK配置/排空、4项SDK配置进程及4项Unix管理端到端测试；保留P1/P2回归，补充代次确认、取消所有权及路由排空单元测试。
- 真实临时CLI闭环通过安装/启用/配置CAS/prepare与commit拒绝/desired2-applied1/离线修订3重启/重载/停用/卸载，SIGTERM退出0。
- 最终Windows GNU全目标严格Clippy交叉检查通过；未运行Windows、arm64、远端CI或真实SnowLuma。不把mock或交叉编译视作目标运行认证。
- 同步AGENT/SKILL、README、ADR0004、安全与兼容性、协议/quickstart和配置/存储/重载/迁移文档；本轮审查问题修复包含OS锁、配置准备态清理、最近确认快照回退、损坏包卸载、drain响应关联和管理落盘失败回退。

### P2 验收记录（2026-10-29）

- Linux x86_64，固定 Rust/Cargo 1.99.1：格式、严格 Clippy、72 项测试及 1 项 rustdoc 示例、警告即错误的文档、十四份 Schema 漂移和空宿主检查通过；实际命令与补跑记录见兼容性文档。
- 使用正式示例实现的子进程闭环、独立 release quickstart 打包/CLI 演示通过；覆盖慢订阅者隔离、嵌套服务、权限/版本拒绝、注册撤销、旧代 BotRef、动作结果未知及 SDK 取消/背压回收。
- 修正握手 minor 协商为双方最小值，P1 无 SDK fixture 显式断言 minor0；保留新版 optional minor 兼容测试。
- 同版本 stable 下 Windows GNU 全目标严格 Clippy 通过；Windows 运行、arm64、远端 CI、真实 SnowLuma 尚未验证。同步项目阶段与兼容性记录，当前未创建 Git 提交。
