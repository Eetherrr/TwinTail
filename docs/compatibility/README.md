# 兼容性与验证记录

本页区分选定的支持目标、尚未执行的检查与已取得的证据。历史 P0 决策见 [ADR 0001](../architecture/0001-p0-baseline.md)，安全限制见[安全边界](../architecture/security.md)，阶段范围见 [SKILL.md](../../SKILL.md)。配置 CI、通过 mock 或外部项目声明均不等于真实平台或 SnowLuma 兼容验证。

## GitHub 0.1.0发行原生CI记录

源码已以MIT公开推送main；v0.1.0 Release由[发行页面](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0)实际状态确认，不将draft写成已正式发布。发行附件仅Linux x86_64。本次全绿[run40901049665](https://github.com/Eetherrr/TwinTail/actions/runs/40901049665)对应head `ad3296b09790e3ea497aac39b41957b66aabf3cd`，以下记录独立于P0–P6历史：

- Linux x64 job123543100297与Linux arm64 job123543100623分别Rust成功184项；默认2项ignored另由显式CLI补跑。各自Python release19/state16通过，后续drift、rustdoc、空host、schema检查通过。
- Windows x64 MSVC job123543100415：Rust157项通过、0失败、0ignored，Python release19通过；后续drift、rustdoc、空host、schema检查通过。Unix state与Linux CLI在Windows跳过，不能将它们列为Windows已执行。
- Web：24项Vitest、4项mock浏览器交互、typecheck/build/asset drift通过，2项live测试跳过；历史本地live结果仍保留于P5记录，不冒充此次远端执行。

这证明列出的native构建/测试范围，不是所有产品行为或长时间运行认证。真实SnowLuma仍仅P4指定部署的正向私聊/重连，reverse未取得真实产品证据。当前矩阵见[平台验证](../release/platforms.md)。

## P6发布加固（Linux本地候选）

用户已授权新建公开GitHub仓库、main分支及MIT下0.1.0发行准备；根许可与元数据由lead同步。本页不将授权写成公开推送或Release已完成，实际发行证据另行记录。发行/状态/合同与soak命令已落盘，见发行指南；最终验证计数不沿用P5历史。状态负责人报告Python3.13.5下16项stdlib unittest、py_compile及真实CLI snapshot/verify/restore/重复restore拒绝通过；最低Python3.11本机未运行，CI等待runner。

P6 Linux本地候选源码与工具验收完成：最终fmt、workspace全目标strict Clippy（-D warnings）和workspace测试通过；清单177项含2项ignored CLI，默认实际执行174项测试+1项doctest。严格rustdoc及protocol/manifest/OpenAPI漂移检查通过。反向测试固定bind失败曾表现为PluginExited/pipeclosed/statefailed，故意占端口已精确复现；原测试reserve端口0后释放、先启动echo，存在并发正向出站连接夺取ephemeral端口的TOCTOU窗口，未捕获原OS占用者，不能声称原始占用实测。仅测试改为p4四项共享async锁、reservation保持到adapter启动前；外部OS竞争窗口缩小但不是原子保留。生产代码未变，适配器19项测试及最终workspace回归通过。既有P4真实SnowLuma范围保持不变，reverse尚无真实SnowLuma证据。

负责人交付的最终payload核验版`dist/p6-soak-report.json`：健康输入分布60秒，总耗时63976毫秒（含过载/清理）；offered6128=128过载+6000健康，delivered6033、dropped35、慢订阅者退休后unrouted60；6000条send_msg逐条核对账号和唯一消息段序号，24代/24次health、历史峰值256，49个直接子进程PID经Linux /proc确认全部消失。此为有限本地mock负载，不证明RSS/内部任务上限或无限常驻，不连接QQ。SDK短p6测试3项通过。

发行负责人报告18项Python测试和真实Linux五binary release构建通过；最终同输入双stage/归档与系统tar解包verify结果以`dist/release-validation.json`记录为准，不在文档嵌入循环变化的archive摘要。最终Linux release宿主另运行2项opt-in CLI均通过：P4 documented流程6.39秒，P5 authenticated HTTP+CLI救援15.55秒；额外2项不计入默认174项+1 doctest。前端npm ci、24项测试、typecheck/build、asset deterministic及npm audit0通过。Windows GNU全目标交叉Clippy通过（最后仅p4测试锁修改不影响生产，精确最终复验另记），仅交叉检查，非MSVC构建或Windows运行；arm64未运行。当前矩阵见[平台验证](../release/platforms.md)，操作与候选边界见[发行入口](../release/README.md)。

## P5 Web 管理验证（2026-10-29，Linux本地验收完成）

lead 报告最终Rust源码检查通过：

- `cargo fmt --all -- --check`、workspace全目标严格Clippy（`-D warnings`）、workspace测试与严格rustdoc构建通过。
- 修复后最终测试清单共166项，包含2项ignored和1项doctest；默认实际执行 **163项测试+1项doctest**，不能把清单数量写成默认全部运行。
- 修复后最终构建显式运行两项opt-in CLI测试均通过：P4 **18.48秒**、P5 **42.53秒**。这些不计入默认163项。
- protocol/manifest Schema与web-admin OpenAPI `--check`漂移检查全部通过。
- Windows x64 GNU以同版本`+stable`（rustc1.99.1）完成全目标严格Clippy交叉检查；仅编译检查，不表示Windows运行验收通过。

前端工具链固定Node26.7.0/npm12.2.1，lead报告最终 `npm ci`、**24项Vitest**、typecheck/build与`npm audit`（0漏洞）通过。真实Chromium对mock后端完成**4项交互测试，13.2秒**，覆盖桌面1440×1050与手机390×844：只读权限、自停提示、启停/配置CAS冲突保留草稿和诊断。执行中无JavaScript错误或横向溢出。截图已生成，但未作人工视觉审查，不将自动化断言描述为视觉审查。

live浏览器验收发现大体积debug包的plugins.installed全量SHA校验阻塞current-thread宿主约10.8秒，导致Web请求超时；修复后的Rust全回归及最终live复验均已通过。管理查询改为有界spawn_blocking（16 permit，由实际完成任务持有）与View短锁快照；plugins.installed改用PackageStore.inventory轻量receipt/manifest结构检查，响应verified:false，不能证明当前包字节完整性。底层list/verify仍全量哈希，安装/enable/reload的校验不降级。lead另报告SDK新增一项duplex回归：4个管理请求pending时ping/health仍可用；已计入修复后最终回归汇总。

集成负责人在修复后真实进程环境并发执行4个管理查询，全部HTTP200：plugins.list约7毫秒、plugins.installed轻量inventory约11毫秒、operations.list约9毫秒、management.overview约8毫秒。四插件均active，真实adapter进程connected=false、不连接QQ；最终真实Chromium live复验在本机45559端口完成 **2/2通过**：桌面16.3秒、手机13.5秒，总运行34.2秒。覆盖在线更新unsupported时保留草稿、stop→离线CAS completed/revision+1→start、reload generation+1且desired=applied、mock错误刷新保留、正式adapter connected=false、安全诊断和logout，无JavaScript错误或横向溢出。P5 Linux本地验收完成，未连接QQ，不改变P4真实部署或Windows运行范围，也未进行人工视觉审查。浏览器结果、Rust/CLI故障恢复与HTTP安全测试分别记录，操作诊断仅有界脱敏journal，不包含stderr。具体操作见[Web管理与CLI救援](../plugin-api/web-admin.md)。

文档负责人另从新私有临时目录执行 `cargo build --locked -p twintail-host -p web-admin`，仅复制binary/manifest/schema，预置revision1环境引用配置，再CLI install/enable：操作completed、插件active、desired/applied均1；静态index200且有CSP、session登录200且cookie HttpOnly、CSRF management.overview200且permissions read/write均true。此次不连接QQ，不代替浏览器关键交互或故障恢复验收。

## 当前 P4 状态

P3 管理、配置与热重载已完成 Linux 本地验收。已有 wire 1.2、版本包存储、配置 Schema/CAS/秘密引用、desired/applied、可查询操作与 exclusive 代次替换。P4 OneBot 类型库和 SnowLuma 正向 Universal WS 适配器已完成 Linux 本地验收及指定真实部署范围验证；P5 Web管理已完成Linux本地验收，最终证据见本文顶部。具体版本、mock及真实逐项证据见 [SnowLuma 专项矩阵](snowluma.md)，运行步骤见[适配器指南](../plugin-api/adapters.md)。

## P4 本地工程验证（2026-10-29）

lead 报告 Linux 工程验收通过：workspace **137 项测试及 1 项 rustdoc 示例**；独立 opt-in CLI smoke **1 项通过**（默认测试集 ignored，不计入137项）。Windows 全目标严格 Clippy 交叉检查通过，包含 real_probe 和 CLI smoke 的最终源码；这不表示 Windows 测试运行通过。P3 以下历史记录保留原阶段计数。

真实验收客户端为 Linux x64、WSL mirrored 模式，远端返回 SnowLuma 2.2.1-node（远端 OS/commit/SHA未知）。直接 Transport 查询/登录/一次私聊通过并获用户收件确认；随后 Supervisor+独立adapter+临时feature 经用户断开/恢复转发，观察 generation1→offline→attempts2…6→generation2，再完成 message→host→feature→send_private_msg 一次回复，status=ok、retcode=0、exit=0，用户确认收到回复。P4 在该指定部署范围验收完成；notice/request、群聊、其他动作、Bearer/TLS及部署OS矩阵仍待测。Windows 原生仅握手诊断，不构成真实客户端运行验收。详见 [SnowLuma 专项记录](snowluma.md)。

## P3 本地验证记录（2026-10-29）

Linux x86_64，Rust/Cargo 1.99.1；本轮完成以下本地验收。工作树尚无 Git 提交。

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 全部通过：**117项测试及1项rustdoc示例**。
- 严格 `RUSTDOCFLAGS=-D warnings` 的 workspace rustdoc、17份 Schema 的两个 exporter `--check`、空宿主 `--check` 全部通过；131个相对文档链接检查通过。
- `cargo +stable clippy --workspace --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings` 通过，覆盖新增配置文件锁、管理 actor 及测试代码的编译；stable 与固定 rustc 同为1.99.1。Windows **仅交叉编译/Clippy**，未执行 Windows 运行、远端CI、arm64或真实SnowLuma。
- 4项 Unix 管理端到端回归通过：本地安装/启停/损坏包卸载、断开客户端后的后台重载/并行查询/幂等指纹冲突、配置秘密引用与有界日志恢复、慢握手期间查询及启动中断恢复。
- 配置存储10项测试、包/计划15项测试、6项独立进程重载测试、4项 SDK 配置进程集成通过。
- 独立进程重载验证端口租约无重叠、候选失败恢复、双重失败可观测、强依赖/权限阻断、取消后 reconcile 续作及 shutdown 回收。SDK 进程测试验证最近确认配置回退和应用 start 拒绝。
- 临时目录真实 CLI 完成 install→enable→在线配置→旧修订拒绝→prepare 拒绝→commit 拒绝→desired2/applied1→stop→离线修订3→start→probe确认3→reload→disable→uninstall；SIGTERM 退出0。该验证使用测试插件，不代表真实 QQ/SnowLuma。

以下保留 P2/P1/P0 历史验证证据。

## P2 本地验证记录（2026-10-29）

Linux x86_64，固定 Rust/Cargo 1.99.1。当前工作树尚无 Git 提交。

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- `cargo test --workspace --locked` 通过；最后新增 SDK 满队列测试后补跑 `cargo test -p twintail-plugin-sdk --test runtime --locked` 通过。合计 **72 项测试及 1 项 rustdoc 示例**：路由单元 7、清单/计划 8、管理 18、CLI 3、P1 进程 12、P2 进程 2、SDK runtime 8 + extra 3、wire 11。
- P2 子进程测试使用正式 echo/mock 实现，覆盖消息段与扩展字段保留、旧 BotRef 不能路由至新实例、嵌套跨插件服务、SemVer 不匹配与权限拒绝。路由测试覆盖慢订阅者独立溢出、注册撤销、固定目标代次、队列满/断连/期限分类；SDK 双工测试覆盖嵌套 RPC、握手/版本、超时、动作结果未知、取消/退出和有界并发。
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`、两个 Schema exporter `--check`、空宿主 `--check` 通过。
- SDK quickstart 在干净的临时 release target 构建宿主与两个正式示例，打包校验后经 CLI 注入/查询，观察到 delivered=1/dropped=0 和一条保留消息段的 send_message，按依赖顺序停用并退出。新增 readiness 探针由进程集成测试使用；不把 `active` 单独视为应用 start 已完成。
- `cargo +stable clippy --workspace --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings` 通过。stable 与固定工具链的 rustc 同为 `1.99.1 (58cf8f9c3 2026-10-13)`。固定工具链本身未安装 Windows std，直接使用该工具链交叉检查报 E0463；以上成功检查使用已安装 target 的同版本 stable。
- Windows **仅交叉编译/Clippy**；未运行 Windows、arm64、远端 CI 或真实 QQ/SnowLuma。

以下保留 P1/P0 历史验证证据，不代表当前功能仍限于该阶段。

## 2026-10-29 本地 P1 验证

Linux x86_64；rustc 1.99.1 (58cf8f9c3 2026-10-13)、cargo 1.99.1 (5f94df478 2026-08-27)。固定 1.99.1 工具链已安装，当前工作树尚无 Git 提交。

- 固定工具链 fmt、全目标严格 Clippy、workspace test、警告即错误的 rustdoc 与五份 Schema 的两个 exporter --check 通过。
- 共 50 项测试及 1 项 rustdoc 示例：wire 10、清单/权限/计划 7、CLI 3、Unix 管理 18、独立插件进程 12。
- 进程测试覆盖取消启动/停止、背压、慢握手期间依赖级联、超时后迟到响应及 generation 递增；stderr/管道 Drop 清理由任务所有权保证，未以测试声称完整进程树控制。
- 真实二进制从临时包完成 --validate、plugins list/status/stop/start，确认 generation 2；SIGTERM 退出码 0、shutdown marker 落盘、管理 socket 已清理。
- `cargo +stable clippy --workspace --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings` 通过，包含 4 项 Windows 管理测试的编译。Windows 上的运行、DACL 跨用户拒绝和 Ctrl-C 尚待实际 runner；不把交叉编译记为运行通过。
- 未执行远端 Actions、arm64 与真实 SnowLuma。P1 只监管直接子进程；仅运行受信任插件。

## P0 工程历史基线

| 项目 | P0 决策 | 验证边界 |
| --- | --- | --- |
| Cargo | 虚拟 workspace，resolver `3` | 唯一成员为 `twintail-host` 二进制 crate |
| Rust | edition `2024`；工具链与当前 MSRV 均为 `1.99.1` | 不承诺更早工具链兼容；其他版本需单独验证 |
| 生命周期 | Tokio；默认持续运行；Ctrl-C；Unix SIGTERM | `--check` 成功退出不替代停止信号验证 |
| 路径 | `--data-dir` 默认 `./data`，`--install-dir` 默认 `./packages`；相对 cwd | P0 仅解析显示，不读写、不创建目录、不解析配置 |
| 插件 / IPC / WebUI | 不在 P0 实现范围 | 无可用接口或兼容性承诺 |

## 当前平台与 CI 矩阵

当前平台执行范围以本页新增GitHub发行记录和[平台验证矩阵](../release/platforms.md)为准：Linux x64、Linux arm64与Windows x64 MSVC已取得native CI证据；state与Linux CLI仅在Linux执行。信号行为、真实SnowLuma与发行附件须分别核对，不能从一般测试通过推断所有部署行为。

以下P0工具链记录及各P0–P6段落保留当时实际执行范围，不因后续native CI而改写历史。

## 2026-10-28 本地 P0 验证

环境：Linux x86_64，`rustc 1.99.1 (58cf8f9c3 2026-10-13)`、`cargo 1.99.1 (5f94df478 2026-08-27)`，由已安装的 `stable` 工具链执行。工作区尚无 Git 提交；记录对应当前 P0 工作树。

- `cargo +stable fmt --all -- --check`：通过。
- `cargo +stable clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `cargo +stable test --workspace --locked`：3 项 CLI 集成测试通过，覆盖启动检查、帮助/版本及非法参数。
- `RUSTDOCFLAGS="-D warnings" cargo +stable doc --workspace --no-deps --locked`：通过。
- 构建后二进制在临时 cwd 使用含空格的自定义路径运行 `--check`：退出码 0，无文件或目录写入。
- 默认驻留进程在报告信号就绪后立即接收 SIGINT 与 SIGTERM，各重复 5 次：均输出停止记录并以 0 退出。
- `cargo +stable tree --workspace --locked`：宿主仅直接依赖 Tokio，无插件实现依赖。

P0 记录时固定版本别名 `1.99.1` 的 rustup 安装仍在下载，因此上述历史证据使用显式 `+stable`；两者预期编译器版本相同，但不把已通过检查描述为已完成固定别名安装。远端 Actions、Windows 和 arm64 的验证状态仍按上表保留。

## SnowLuma / OneBot v11 矩阵

SnowLuma 独立部署，adapter-snowluma 连接其 OneBot 服务，宿主不内嵌 SnowLuma。P4 正向 Universal WS、单账号、Bearer、四类事件和未知扩展、echo/pending、有界重连及停止均已实现；类型夹具、mock WS 与独立进程证据列于 [SnowLuma 专项矩阵](snowluma.md)。Linux 本地验收及指定真实部署的重连后宿主业务私聊闭环已通过；其余真实能力按专项矩阵保留待测。

P6已实现loopback reverse Universal，本地mock/子进程证据不等同真实产品兼容；split API/Event 和 HTTP 传输仍延期。真实产品具体动作、事件、平台和重连行为只按专项矩阵取得的证据填写，不使用“完整支持 OneBot v11”概括未测范围。历史 P0–P3 记录保留当时的功能与验证范围。

## 后续记录要求

P4 开始为每个真实集成记录 SnowLuma 版本/提交、TwinTail 版本/提交、操作系统与架构、传输方式、鉴权模式及测试日期。记录测试的事件与动作、字段差异、扩展、缺失行为、重连结果和明确限制。真实链路至少验证收消息→业务插件→发消息及断线后恢复，并分别记录失败与结果未知的动作语义。

无凭据 CI 使用脱敏夹具和 mock，验证协议处理与故障路径；真实 QQ/SnowLuma 集成单独执行并标注证据来源。夹具不含有效 token、账号秘密或未脱敏消息，不以 mock 通过替代真实产品兼容结论。

## 规范与产品入口

- [OneBot v11 规范](https://github.com/botuniverse/onebot-11)
- [正向 WebSocket 规范](https://github.com/botuniverse/onebot-11/blob/master/communication/ws.md)
- [反向 WebSocket 规范](https://github.com/botuniverse/onebot-11/blob/master/communication/ws-reverse.md)
- [SnowLuma 仓库与 README](https://github.com/SnowLuma/SnowLuma)

外部公开元数据的本次查询结果见专项矩阵；公开声明不证明已运行或验证产品。
