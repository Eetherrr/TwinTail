# 哪些平台实际测过？

下载前先核对操作系统和 CPU 架构，再看下面证据。**源码能通过测试、目标程序能启动、真实 QQ 业务可用、已有下载附件，是四个不同结论。** v0.1.1 已公开 Linux x86_64、Linux aarch64 与 Windows x86_64 MSVC 附件；v0.1.0 历史附件仍仅 Linux x86_64。

## 这些缩写怎么读

| 名词 | 含义 |
| --- | --- |
| x64 / x86_64 | 常见 Intel/AMD 64 位 CPU |
| arm64 / aarch64 | 64 位 ARM CPU，不与 x64 二进制互换 |
| GNU / MSVC | 构建目标采用的工具链与系统 ABI；Windows GNU 与 MSVC 记录分别看 |
| 原生 / native | 在目标系统和架构上构建或执行，具体以实际命令为准 |
| 交叉检查 | 在另一环境为目标检查或编译；不代表已在目标机器运行 |
| CI | 提交后由自动化执行的检查；默认没有真实 QQ 账号 |

## 怎么用这张表

1. 确认自己的系统与架构，与下表 target 对上。
2. 看源码提交和日期：网站跟随 main，下载版本可能更旧。
3. 看“限制”列：需要真实 SnowLuma、live 浏览器或数据备份时，继续查专项记录。
4. 部署后仍按[运维指南](operations.md)进行本机受控启动，不只看表格里的“通过”。

## 最新源码回归（2026-11-08）

[run 42476673350](https://github.com/Eetherrr/TwinTail/actions/runs/42476673350) 对源码 `8c3d096a88cdf40d77d1c3ce99b3ce3bd934d800` 的四个任务全部通过：Linux x64（job128719468100）、Linux arm64（job128719467917）、Windows x64 MSVC（job128719468070）、WebUI（job128719468130）。该提交包含插件 README、彻底移除及 Windows 回归修复。Rust 任务包含格式、严格 Clippy、工作区测试、构建、文档与生成合同漂移检查；Linux 额外执行显式 CLI smoke 和状态工具测试，Web 执行单元/模拟浏览器与构建检查。这里只记录任务成功，不挪用旧运行的测试计数。

Windows 原生通过不代表 Windows 真实 QQ/SnowLuma 业务、反向真实部署、浏览器 live 用例或 Windows 备份工具已验证。公开 Release 是对应版本 tag 的独立源码快照，不自动包含之后 main 的新增功能。

## v0.1.0 发行时历史证据

发行时GitHub native CI已通过：[run40901049665](https://github.com/Eetherrr/TwinTail/actions/runs/40901049665)，源码head `ad3296b09790e3ea497aac39b41957b66aabf3cd`。以下是本次运行范围，更早的本地专项证据仍见兼容记录；CI通过不扩大真实SnowLuma验证或发行附件范围。

| 目标 | 本次native执行证据 | 限制 |
| --- | --- | --- |
| Linux x64 GNU | job123543100297，Rust成功184项（默认2项ignored由显式CLI补跑），Python release19/state16；后续drift/doc/空host/schema检查通过 | v0.1.0附件仅此平台；60秒soak与真实SnowLuma另按本地专项记录 |
| Linux arm64 GNU | job123543100623，Rust成功184项（默认2项ignored由显式CLI补跑），Python release19/state16；后续drift/doc/空host/schema检查通过 | 已有native测试，不表示提供arm64发行附件或真实SnowLuma部署验证 |
| Windows x64 MSVC | job123543100415，Rust157项通过、0失败、0ignored，Python release19；后续drift/doc/空host/schema检查通过 | state工具与Linux CLI在此平台跳过；无Windows发行附件，不代表真实SnowLuma运行 |
| Windows x64 GNU（历史交叉检查） | 历史全目标严格Clippy交叉检查通过 | 与本次MSVC native证据分别记录 |

同次Web检查：24项Vitest、4项mock浏览器交互、typecheck/build与asset drift通过；2项live浏览器测试跳过。本次不将历史本地live通过当作远端执行。源码main已公开，v0.1.0发行是否发布以[Release页面](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0)实际状态为准。当时待发布包的同输入归档与解包报告仍见dist/release-validation.json。

Linux本地证据为开发机x64，真实SnowLuma验证客户端为WSL mirrored。远端报告2.2.1-node，服务OS/commit/SHA未知，Windows ServerBox仅SSH转发。不得把远端OS列为Windows或据此宣称reverse/其他动作已验。

后续每条验证结果须附日期、target triple、Rust/Node版本、源码/包摘要、命令、实际执行项数、运行环境与限制。release包能生成、SHA能校验、目标能启动以及完整业务可用是不同结论。历史阶段具体证据见[兼容记录](../compatibility/README.md)。


## 新发行自动化的状态

v0.1.1 [发行运行 42577373476](https://github.com/Eetherrr/TwinTail/actions/runs/42577373476)，源码 `cf62e4381600a383d70fbba44b11811e75d5558f`：完整 Rust/WebUI 四任务、Linux x86_64（job129030719639）、Linux arm64（job129030719692）及 Windows MSVC（job129030719483）的原生 release 构建、宿主 `--version`/`--check`、归档校验均成功，汇总与草稿创建成功。首次公开验证因草稿按 tag 查询返回错误而保留草稿；[恢复运行 42580369848](https://github.com/Eetherrr/TwinTail/actions/runs/42580369848) 已复核原始包并公开。该恢复运行末尾对公开元数据的比较仍报失败，不能将其整体记为绿色；独立匿名下载已确认 Release 为 public/latest，八附件服务端 SHA256、七项下载摘要及三个归档均校验通过。没有新增真实 SnowLuma 或 Windows live 浏览器结论。详见[自动化发行](automation.md)。


v0.1.1 公开附件回下载记录（2026-11-09）：

| 归档 | 大小（字节） | SHA256 |
| --- | ---: | --- |
| `twintail-0.1.1-linux-x86_64.tar.gz` | 7539703 | `0c02cab6d70b7ec86eab4ec4a6a297d828823ce116dbb0aba87776bcd907bc58` |
| `twintail-0.1.1-linux-aarch64.tar.gz` | 7413169 | `dfd11a556a75ad52095457a704c0687cade338530b788c33d0b9ae0ac914b61e` |
| `twintail-0.1.1-windows-x86_64.zip` | 6311804 | `297ecdb563294e9e8eebc8193e434e1681feb440a42a9ce7db398f65f643921d` |

三份验证报告均保持 `runtime_verified: false`，不把归档结构校验写成真实 QQ 业务验证。另在 Linux x86_64 本机解包下载附件，目录再次 verify、宿主 `--version`（0.1.1）及 `--check` 均通过；未连接 QQ。
