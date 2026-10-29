# GitHub 0.1.0发行准备

P6 Linux本地候选完成；用户已授权新建公开GitHub仓库、使用main分支并以MIT准备0.1.0发行。源码main已公开推送；v0.1.0 GitHub Release状态以[发行页面](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0)为准，draft不等同正式发布；cargo publish不属于本次GitHub发行流程。工具命令已按实际接口核对，最终运行结果由lead统一验收。

| 文档 | 使用场景 |
| --- | --- |
| [用户运维](operations.md) | 首次部署、健康判断、升级与故障恢复 |
| [插件发布](plugins.md) | 插件候选构建、合同、包检查和迁移 |
| [安全与依赖升级](security-updates.md) | MIT发行许可、依赖变更与秘密处理 |
| [平台验证矩阵](platforms.md) | 区分实现、交叉检查和目标平台运行证据 |

现有[真实SnowLuma范围](../compatibility/snowluma.md)保持P4指定部署结论，不扩展为reverse或其他OS认证。P5浏览器本地验收也不表示真实QQ动作通过。

项目许可证已由用户决定为MIT；根LICENSE与Cargo许可元数据由lead同步，制品纳入相应许可文件后重新stage/verify。第三方依赖的版权与再分发义务仍独立核对，MIT决定不扩大平台或真实产品验证范围。

发行工具已落盘：`python3 tools/release/release.py --help`，build/stage/archive/verify用法见[插件候选](plugins.md)。输出固定标记candidate、public_release=false、runtime_verified=false；目标运行与许可判断由独立报告给出，不能从摘要通过推断。

合同工具见[合同与有界soak](../plugin-api/contract-tools.md)。

候选安全审查与未解决gate见[依赖与安全审查](dependency-audit.md)。

候选包含binary、白名单文档和工具脚本，不是完整源码checkout。release build与Cargo合同/soak示例命令必须在同版本完整源码仓库运行；不能进入解包目录后直接执行Cargo命令并期待缺失源码可构建。state脚本仅依赖Python标准库，仍受Unix/停机边界约束。
