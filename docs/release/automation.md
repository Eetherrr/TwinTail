# 用 GitHub Actions 准备新版本发行

本页给项目维护者说明如何让 GitHub 自动构建下载包，再由人决定是否公开。普通用户下载现有版本请看[发行入口](README.md)。

**v0.1.1 已公开，提供三个平台的下载包。** [首次运行 42577373476](https://github.com/Eetherrr/TwinTail/actions/runs/42577373476) 对应源码 `cf62e4381600a383d70fbba44b11811e75d5558f`，完整 CI、三平台打包、汇总和草稿创建成功；公开前按 tag 读取草稿失败。[恢复运行 42580369848](https://github.com/Eetherrr/TwinTail/actions/runs/42580369848) 下载原始 review bundle 并逐字节核对八个既有附件后公开，版本 tag 和原附件未变。恢复运行末尾的元数据比较仍报失败，不能把该运行记为整体成功；随后独立匿名下载确认 [Release](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.1) 为 public/latest，八个服务端摘要、七项 SHA256SUMS 及三个归档结构均一致。公开流程现允许 GitHub 将草稿下载 URL 转换为正式 tag URL，仍严格检查附件身份和内容。平台证据见[平台矩阵](platforms.md)。

## 先认识三个词

- **Actions**：GitHub 上按仓库工作流执行命令的服务，不在你的电脑上运行。
- **artifact**：一次运行临时保存的构建结果，需要到 Actions 下载；它不是已公开发行附件。
- **Release 草稿**：尚未公开的发行页面。默认自动化只准备草稿，由维护者点击 Publish release；也可在创建新附注 tag 时显式批准验证后自动公开。

## 选择入口：手动试构建，或 tag 准备草稿

`.github/workflows/release.yml` 提供以下入口：

| 入口 | 行为 |
| --- | --- |
| 推送普通 `v*` tag（轻量 tag 或无批准行的附注 tag） | 校验版本 → 复用完整 CI → 三平台原生 release 构建与打包 → 汇总摘要 → 创建 GitHub Release **草稿** |
| 推送附注消息含独立精确行 `TwinTail-Publish: approved` 的新 `v*` tag | 相同验证及草稿创建 → 下载并核对全部附件 → 验证成功后自动**公开并设为 latest** |
| Actions → Release candidates → Run workflow | 输入 `vVERSION`，对选定 ref 执行相同校验、构建及汇总，仅上传 Actions artifacts，**不创建 Release** |

手动入口输入的是预期版本，不是额外 checkout 参数；源码由 Run workflow 选择的 ref 决定。tag 必须严格匹配 workspace 与四个插件清单的版本；支持 `v1.2.3` 和 `v1.2.3-rc.1` 等 SemVer，当前底层打包器不支持 `+build` 元数据，前置校验直接拒绝。非法版本在构建前失败。不自动改版本、不自动打 tag、不发布 Cargo registry，不修改现有 v0.1.0。

## 第 1 步：准备源码与版本

1. 更新 workspace 和四个官方/示例插件清单版本，以及适用的锁文件、前端版本与文档示例。
2. 将 CHANGELOG 的相关 Unreleased 变更整理为正式版本及迁移说明，检查许可、第三方声明和兼容矩阵。
3. 保证 WebUI 打包静态资源已随源码同步；工作流复用 CI 的 npm 锁定安装、测试、构建与资源漂移检查。
4. 提交源码，先用手动入口验证选定提交；审核三个平台附件、版本及行为。
5. 对同一源码创建并推送新 tag。不要移动已有 tag 或复用旧版本名。

```sh
# VERSION 替换为源码实际版本，例 0.2.0；不应重复推送已发布的 v0.1.0。
git tag -a vVERSION -m "TwinTail vVERSION"
git push origin vVERSION
```

### 可选：明确批准自动公开

若操作者已决定该新版本应在全部检查通过后直接公开，创建附注 tag 时加入以下精确独立行。该选择由操作者作出，工作流不会自动创建、移动或覆盖 tag：

```sh
# VERSION 替换为尚未存在、与源码版本一致的新版本号。
git tag -a vVERSION -m "TwinTail vVERSION" -m "TwinTail-Publish: approved"
git push origin vVERSION
```

只有**远端最顶层附注 tag 的消息**中精确的 `TwinTail-Publish: approved` 独立行有效；大小写、前后空格、前缀/后缀都不等价。轻量 tag、普通附注 tag、嵌套 tag 内层消息、提交消息和手动构建均不构成公开批准。不要给预发行版本加此行，除非确实希望它公开并成为 latest；本入口不自动推断 pre-release。

默认示例仍只创建草稿。此标记不跳过版本校验、完整 CI、原生构建、打包或草稿审核数据验证，也不覆盖同名现有 Release；需要保留人工审核时省略批准行。

## 第 2 步：在 Actions 查看构建结果

手动预检时，在仓库 Actions → Release candidates → Run workflow 中选择源码 ref，填写与源码一致的 `vVERSION`。运行应先完成版本校验，再进行 CI 与三个平台构建；任何红色失败任务都应先展开日志定位，不直接重复创建 tag。

若工作流列表尚未出现，先确认工作流已经进入远端所选分支；本地保存 YAML 不会让 GitHub 自动看到它。

完整 CI 复用仓库已有 Rust/WebUI 检查。构建分别使用 Linux x86_64、Linux aarch64、Windows x86_64 MSVC 原生 runner，不使用跨编译结果冒充目标运行。每个平台重新执行 release 构建，运行宿主 `--version`、无副作用 `--check`，随后 stage、archive、verify。

Artifacts 包括：

- `candidate-linux-x86_64`、`candidate-linux-aarch64`、`candidate-windows-x86_64`：各平台归档和结构验证 JSON。
- `release-review-bundle`：三个归档、分平台验证 JSON、外部 `SHA256SUMS` 和生成的发行说明。

Actions artifacts 保留 14 天。tag 运行将汇总文件上传到新建 Release 草稿，正式 Release 附件不受此 Actions 保留期限影响。草稿只会在所有检查和平台包成功后创建；创建前重新解析远端轻量/附注 tag，核对其提交仍等于本次源码 SHA。请在仓库设置中保护版本 tag，校验与创建之间不是原子操作。现有 Release（包括草稿）会阻止覆写。并发发布不自动取消，避免中途取消上传。

汇总重新调用打包工具校验三个归档，拒绝缺失、多余、重复或链接输入，并计算外部下载摘要。验证报告保留 `runtime_verified: false`：归档结构与摘要不是来源认证、签名或完整业务运行证明。原生源码测试及宿主启动 smoke 与归档静态验证是不同证据。

## 第 3 步：默认人工公开，批准 tag 验证后自动公开

对于带明确批准行的 tag，工作流仍先创建草稿，再通过 Release 列表定位唯一同名草稿并验证附件状态；附件必须精确为 **8 个文件**：三个版本/平台归档、三个 `TARGET-validation.json`、`SHA256SUMS`、`release-notes.md`。随后按附件数字 ID 下载到临时目录，与 `release-output` 的文件集合和内容逐字节比较，并对本地与下载副本分别运行严格 SHA256 校验；摘要必须且仅覆盖其余七个文件。`target_commitish` 是 GitHub 的描述字段，不作为提交绑定依据：公开前实际解析远端 tag，确认顶层批准行及最终提交仍匹配受测源码，并确认草稿数字 ID 未变，才按该 ID PATCH `draft: false` 与 `make_latest: true`。任一前置检查失败都会停止，保留草稿供人工检查；不会自动删除、覆盖或修补附件。远端检查与公开不是原子事务，仍需保护 tag 和限制并发人工编辑。

普通 tag 保持以下人工审核流程。

在 GitHub Releases 打开草稿，确认：

- tag 与预期源码提交一致，三个目标及 SHA256 齐全；
- 按 CHANGELOG 补齐用户可读变更、升级/数据迁移说明、已知问题；
- 在目标环境按需解包、再校验并执行部署/升级测试；
- 更新验证范围，不把 mock、宿主空启动或 CI 当真实 QQ/SnowLuma 验收；
- 预发行版本按需勾选 pre-release，确认无误后点击 Publish release。

默认令牌仅需要仓库内的读权限，以及仅 tag push 执行的独立草稿发布 job 的 `contents: write`；不需要个人 PAT。仓库或组织若禁用工作流写入，请由管理员按策略调整。首次创建失败但已留下草稿时，先检查附件是否完整，不自动覆盖或删除；人工处理后再决定重跑，避免改写既有发布。

## v0.1.1 既有草稿的独立恢复

首次公开验证失败后，`.github/workflows/release-recovery.yml` 提供独立恢复，不重新执行打包、不重新上传、不创建或删除 Release，也不移动版本 tag。它只接受已经批准的 `v0.1.1` 与源码 `cf62e4381600a383d70fbba44b11811e75d5558f`，默认原始运行是 `42577373476`，不是恢复运行自己的 ID。

维护者可在 Actions → Recover approved existing release draft 输入上述 tag 与原始 run；推送单独的 `release-recover-v0.1.1` tag 也可触发恢复代码；重复尝试使用新标签（如 `release-recover-v0.1.1-attempt2`），不移动旧标签，恢复目标始终固定为 v0.1.1。恢复 tag 指向包含修复的维护提交，版本 tag 仍指向原发行源码。恢复会读取并脱敏输出原失败 job 日志，核对原运行中四个完整检查、三个原生包、汇总及草稿创建全部成功，且唯一失败位于公开验证步骤；下载原运行的 `release-review-bundle`，重新验证三个归档、报告及七项摘要，再下载现有草稿的八个附件逐字节比较。再次核对批准 tag、草稿身份和附件未变后，唯一远端修改是按草稿 ID 公开并设为 latest。

任何检查不符都会停止；公开请求发生网络异常时不自动重试，应先查看远端状态。此入口是本次固定版本的恢复工具，不是绕过完整发行流程的通用发布入口。

## 本地验证与限制

```sh
python3 -m unittest discover -s tools/release -p 'test_*.py'
python3 tools/release/automation.py validate --tag v0.1.1
```

`collect` 需要三平台目录，每个目录含一个匹配版本的归档及 `validation.json`，详见工具帮助；输出必须是新目录。

工作流文件进入远端后才能在 Actions 运行。仅本地静态校验不代表三平台 Release job 已实跑；工作流不自动创建版本 tag 或覆盖旧附件；仅操作者选定的明确批准附注 tag 可在全部验证成功后触发公开。


## 失败时先查哪一层

| 失败位置 | 检查方法 |
| --- | --- |
| 版本校验 | tag 是否与 workspace 和四个插件清单一致，是否含不支持的 `+build` |
| CI / 构建 | 打开失败 job 的实际日志；不要将其他提交的绿色结果当成本次结果 |
| 汇总 | 三个目标的归档与 validation.json 是否齐全，版本和布局是否一致 |
| 草稿创建 | 目标 tag 是否仍指向同一提交，是否已存在同名 Release，仓库是否允许 job 写入 |
| 自动公开验证 | 顶层批准行、草稿目标/附件集合、下载字节及 SHA256 是否一致；失败留在草稿，不跳过校验强行重跑 |
| 上传后中断 | 先检查现有草稿与附件，再人工决定处置；不自动覆盖现有发行 |

成功标志应分别记录：Actions 运行 URL 与提交、三个归档及摘要、草稿链接，最后才是人工公开或明确批准自动公开后的 Release 链接。仅有本地单元测试或 YAML 校验结果不属于远端实跑成功。
