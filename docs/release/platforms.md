# 平台验证矩阵

P6 Linux本地候选完成。本表分别记录实现、交叉检查和运行证据；未测项不从其他平台外推，各项证据分别如下，最终归档与解包校验报告为dist/release-validation.json。

| 客户端目标 | 实现/构建状态 | 已有运行证据 | P6新增发行/恢复/合同/soak |
| --- | --- | --- | --- |
| Linux x64 GNU | release目标linux-x86_64（x86_64-unknown-linux-gnu）已实现 | P5本地Rust/CLI/Chromium已验收；P4指定真实部署见专项记录 | 真实五binary release构建、state16/release18测试及60秒payload soak通过；adapter19项、最终174项默认测试+1 doctest及strict checks通过；端口测试修复生产不变，最终制品报告单独记录 |
| Linux arm64 GNU | release目标linux-aarch64（aarch64-unknown-linux-gnu）布局已实现 | 未运行 | 实际目标构建/运行待验，不标支持通过 |
| Windows x64 MSVC | release目标windows-x86_64（x86_64-pc-windows-msvc）与.exe清单重写已实现 | 未运行 | 实际MSVC构建/运行待验；state工具明确不支持Windows |
| Windows x64 GNU（历史检查） | 管理命名管道和平台代码已实现，P5全目标严格Clippy交叉检查通过 | 未运行；原生HTTP握手诊断不是完整宿主/adapter运行验收 | 不等同MSVC发行构建或目标运行 |

Linux本地证据为开发机x64，真实SnowLuma验证客户端为WSL mirrored。远端报告2.2.1-node，服务OS/commit/SHA未知，Windows ServerBox仅SSH转发。不得把远端OS列为Windows或据此宣称reverse/其他动作已验。

每条P6结果须附日期、target triple、Rust/Node版本、源码/包摘要、命令、实际执行项数、运行环境与限制。release包能生成、SHA能校验、目标能启动以及完整业务可用是不同结论。历史阶段具体证据见[兼容记录](../compatibility/README.md)。
