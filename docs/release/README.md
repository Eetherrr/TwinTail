# 发行与运维

TwinTail 采用 MIT，源码位于公开仓库的 main 分支，[v0.1.0 Release](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0) 已发布 Linux x86_64 附件及校验记录。main 的后续功能以 CHANGELOG 的 Unreleased 为准，不自动进入已有附件。本文介绍部署、插件发布和候选校验；Cargo registry 发布不属于现有 GitHub 发行流程。具体平台和真实 SnowLuma 范围分别以验证矩阵及兼容性记录为准。

| 文档 | 使用场景 |
| --- | --- |
| [用户运维](operations.md) | 首次部署、健康判断、升级与故障恢复 |
| [插件发布](plugins.md) | 插件候选构建、合同、包检查和迁移 |
| [安全与依赖升级](security-updates.md) | MIT发行许可、依赖变更与秘密处理 |
| [平台验证矩阵](platforms.md) | 区分实现、交叉检查和目标平台运行证据 |

现有[真实SnowLuma范围](../compatibility/snowluma.md)保持P4指定部署结论，不扩展为reverse或其他OS认证。P5浏览器本地验收也不表示真实QQ动作通过。

项目根 LICENSE、Cargo 许可元数据与发行清单已采用 MIT；再发布制品时须保留项目许可和第三方声明，并重新执行 stage/verify。第三方依赖的版权与再分发义务仍独立核对，MIT决定不扩大平台或真实产品验证范围。

发行工具已落盘：`python3 tools/release/release.py --help`，build/stage/archive/verify用法见[插件候选](plugins.md)。输出固定标记candidate、public_release=false、runtime_verified=false；目标运行与许可判断由独立报告给出，不能从摘要通过推断。

合同工具见[合同与有界soak](../plugin-api/contract-tools.md)。

候选安全审查与未解决gate见[依赖与安全审查](dependency-audit.md)。

候选包含binary、白名单文档和工具脚本，不是完整源码checkout。release build与Cargo合同/soak示例命令必须在同版本完整源码仓库运行；不能进入解包目录后直接执行Cargo命令并期待缺失源码可构建。state脚本仅依赖Python标准库，仍受Unix/停机边界约束。
