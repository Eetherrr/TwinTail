# 平台验证矩阵

当前GitHub native CI已通过：[run40901049665](https://github.com/Eetherrr/TwinTail/actions/runs/40901049665)，源码head `ad3296b09790e3ea497aac39b41957b66aabf3cd`。以下是本次运行范围，P0–P6历史本地证据仍见兼容记录；CI通过不扩大真实SnowLuma验证或发行附件范围。

| 目标 | 本次native执行证据 | 限制 |
| --- | --- | --- |
| Linux x64 GNU | job123543100297，Rust成功184项（默认2项ignored由显式CLI补跑），Python release19/state16；后续drift/doc/空host/schema检查通过 | v0.1.0附件仅此平台；60秒soak与真实SnowLuma另按本地专项记录 |
| Linux arm64 GNU | job123543100623，Rust成功184项（默认2项ignored由显式CLI补跑），Python release19/state16；后续drift/doc/空host/schema检查通过 | 已有native测试，不表示提供arm64发行附件或真实SnowLuma部署验证 |
| Windows x64 MSVC | job123543100415，Rust157项通过、0失败、0ignored，Python release19；后续drift/doc/空host/schema检查通过 | state工具与Linux CLI在此平台跳过；无Windows发行附件，不代表真实SnowLuma运行 |
| Windows x64 GNU（历史交叉检查） | P5/P6全目标严格Clippy交叉检查通过 | 与本次MSVC native证据分别记录 |

同次Web检查：24项Vitest、4项mock浏览器交互、typecheck/build与asset drift通过；2项live浏览器测试跳过。本次不将历史本地live通过当作远端执行。源码main已公开，v0.1.0发行是否发布以[Release页面](https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.0)实际状态为准。候选同输入归档与解包报告仍见dist/release-validation.json。

Linux本地证据为开发机x64，真实SnowLuma验证客户端为WSL mirrored。远端报告2.2.1-node，服务OS/commit/SHA未知，Windows ServerBox仅SSH转发。不得把远端OS列为Windows或据此宣称reverse/其他动作已验。

每条P6结果须附日期、target triple、Rust/Node版本、源码/包摘要、命令、实际执行项数、运行环境与限制。release包能生成、SHA能校验、目标能启动以及完整业务可用是不同结论。历史阶段具体证据见[兼容记录](../compatibility/README.md)。
