# 手工验证与排障

当前状态为 **P1 Linux 本地验收通过**。以下流程用于 Unix 本地验证，依赖已安装的固定 Rust 工具链；Windows 本机管理已实现并通过交叉编译/Clippy，Windows 运行验证待执行。

## 无 SDK 测试 fixture

仓库提供 [p1-fixture.rs](../../crates/twintail-host/src/bin/p1-fixture.rs)，直接编码 wire JSON，供协议与故障注入测试使用，**不是正式插件样例或 SDK quickstart**。当前默认参数为 mode `normal`、id `fixture`；手工清单应显式对齐。使用仓库根目录下的新测试目录：

```sh
cargo build -p twintail-host --bins --locked
mkdir -m 700 p1-data
mkdir -p p1-packages/fixture
cp target/debug/p1-fixture p1-packages/fixture/p1-fixture
cat > p1-packages/fixture/plugin.toml <<'TOML'
id = "fixture"
name = "P1 protocol test fixture"
version = "0.1.0"
protocol_version = 1
host_api = "^0.1.0"
entrypoint = "p1-fixture"
args = ["normal", "fixture"]
capabilities = []
required_capabilities = []
TOML
cat > p1-data/host.toml <<'TOML'
[[plugins]]
path = "fixture"
grants = []
TOML
target/debug/twintail-host --validate --data-dir ./p1-data --install-dir ./p1-packages
target/debug/twintail-host --data-dir ./p1-data --install-dir ./p1-packages
```

另开终端，同一仓库目录执行：

```sh
target/debug/twintail-host plugins list --data-dir ./p1-data
target/debug/twintail-host plugins status fixture --data-dir ./p1-data
target/debug/twintail-host plugins stop fixture --data-dir ./p1-data
target/debug/twintail-host plugins start fixture --data-dir ./p1-data
```

list/status 输出 JSON outcome；stop 后状态 stopped，再 start 会增加 generation。宿主 Ctrl-C / Unix SIGTERM 时停止插件并清理自己的管理 socket。fixture 解析 initialize 提供的数据目录，配合 shutdown 写入 `plugins/fixture/data/fixture-stopped`。它还提供 wrong-version、oversize、bad-json、handshake-timeout、crash、permission、granted、duplicate-id、unknown-response、response-timeout 等测试模式；具体断言以集成测试为准，不将故障模式当作生产插件模板。

## 常见问题

| 现象 | 检查与恢复 |
| --- | --- |
| `--check` 成功但插件未验证 | 它只做引导检查，不读配置、不创建目录；用 `--validate` 做离线包验证 |
| 显示 validated 0 | host.toml 不存在就是空计划；检查 data-dir 和 cwd |
| 数据目录权限错误 | Unix data-dir 必须为当前有效用户拥有且精确 0700；已有目录不会自动 chmod；检查后由操作者修正 |
| 祖先目录不安全或含链接 | 管理通道拒绝符号链接、`..`、不可信所有者及不安全的组/其他用户可写祖先；root 所有的 sticky 临时目录有受限例外 |
| management endpoint already exists | 先确认没有宿主运行；确认归属和路径后手动清理残留 `DATA/management.sock`，宿主不会自动删除既有端点 |
| 无法执行插件 | 核对入口平台、文件类型和 Unix 可执行位；入口必须留在包根内 |
| 握手失败 | 核对 hello 的 id、version、必需能力；stdout 只能输出合法帧，ready 后等待 activate |
| failed 不自动恢复 | P1 自动重启预算 0；修正后显式 start，先恢复强依赖 |
| stop conflict | 先 stop 活动依赖者，再 stop 提供者 |
| 插件日志不出现 | P1 持续读取并丢弃 stderr，只使用固定 4096 字节缓冲；没有日志检索、脱敏展示或完整速率/CPU 限制 |
| 管理操作超时 | 网络读写默认 5 秒，操作等待期限独立且以代码为准；先 status，确认结果后再决定是否重试 |

Unix 管理帧最多 1 MiB，管理连接并发最多 16；socket 权限 0600，客户端和服务端检查 peer UID 与宿主有效用户一致。每连接只处理一次请求。目录私有性与 peer UID 不隔离同用户恶意插件。插件必须遵守直接子进程生命周期范围，禁止派生守护进程。
