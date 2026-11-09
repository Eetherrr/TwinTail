# 部署、日常检查与备份恢复

本页帮助你把已构建的 TwinTail 程序放到自己的机器上运行，以及在升级或故障时保住数据。第一次只是体验，可先看[无 QQ 示例](../plugin-api/quickstart.md)；需要管理网页或真实接入时，另看 [WebUI 安装](../plugin-api/web-admin.md)与[适配器安装](../plugin-api/adapters.md)。

## 部署前准备什么

- 匹配操作系统与 CPU 的程序。v0.1.1 已公开 Linux x86_64、Linux aarch64 与 Windows x86_64 MSVC 预编译附件；按实际系统与 CPU 选择。
- 一个由运行用户独占的目录，保存安装文件；另一个私有目录保存配置和数据。不要混用已有生产目录做练习。
- 运行宿主的终端或服务环境，以及本机命令行恢复入口。使用秘密环境引用时，变量要交给宿主本身，不能只设置在后来发 CLI 命令的终端。
- 若接 QQ：先自行部署和登录 SnowLuma，取得其 OneBot 服务实际配置。它不是 TwinTail 归档的一部分。

**宿主**是 `twintail-host`；**安装根**放插件版本包；**数据根**保存启用计划、配置、修订和业务数据。所谓“升级程序”通常只替换前者，不应顺带覆盖后者。

## 预编译包：第一次检查与空宿主启动

1. 从[正式发行页面](https://github.com/Eetherrr/TwinTail/releases)下载，比较外部发布摘要。可在 Linux 用 `sha256sum 下载文件名`；摘要相同只证明内容匹配，不证明来源可信。
2. 用对应版本的校验工具检查归档，再解包至新的私有目录，解包后再校验。校验工具入口如下（`ARCHIVE`、`UNPACKED` 换成实际路径）：

   ```sh
   python3 tools/release/release.py verify ARCHIVE
   python3 tools/release/release.py verify UNPACKED
   ```

   首次尚未解包时，可从对应版本完整源码取得工具；不要先执行包内二进制来“验证”不可信下载。校验本身不执行二进制。
3. 在解包顶层运行下面命令，安装根使用包内已有的 `packages`。示例是 Bash/Linux：

   ```sh
   umask 077
   mkdir -p ./data
   chmod 700 ./data
   ./bin/twintail-host --version
   ./bin/twintail-host --check --data-dir ./data --install-dir ./packages
   ./bin/twintail-host --validate --data-dir ./data --install-dir ./packages
   ./bin/twintail-host --data-dir ./data --install-dir ./packages
   ```

4. 最后一条保持前台运行；另开终端在同一解包目录查询：

   ```sh
   ./bin/twintail-host plugins list --data-dir ./data
   ```

   **没有 `host.toml` 时列表为空是正常的**：包不会替你启用插件、授予权限或生成账号配置。`--check` 不读写运行状态，`--validate` 只校验包和启用计划，不证明配置 Schema、秘密环境或业务初始化成功。Ctrl-C 可有序停止宿主。
5. 按 [WebUI](../plugin-api/web-admin.md)或[适配器](../plugin-api/adapters.md)指南先预置必填配置，再启用所需包。归档里的版本包已经位于安装根，不要将相同版本重复安装到相同位置；需要导入外部本地包时才使用 `plugins install`。预编译包没有完整源码，Cargo 示例需同版本完整源码。

后续示例中的 `twintail-host` 假设已加入 PATH；否则使用程序实际路径，例如 `./bin/twintail-host`。Windows 使用 `.exe` 与私有 ACL，不运行上面的 Unix 权限命令。

## 运行权限与首次配置

使用私有数据根与独立安装根，运行身份拥有最小必要的文件/网络权限。安装包只存版本制品，配置、desired/applied修订、操作记录和业务数据留在数据根。先检查包目标平台、来源和摘要；摘要用于损坏检测，不是签名或来源认证。只运行受信任插件，独立进程不是沙箱。

从源码打包和首次配置的详细入口：[Web管理](../plugin-api/web-admin.md)、[正向适配器](../plugin-api/adapters.md)、[无QQ示例](../plugin-api/quickstart.md)。Windows包须使用.exe入口，安装后不原地修改清单/二进制。宿主启动前注入环境秘密引用，浏览器不写入明文token。无配置的空计划不是错误。

## 每次如何判断运行正常

同时判断进程active、配置desired/applied、外部连接connected与业务响应；单项成功不能替代其他层。写操作先接纳再后台执行，保存operation id并查询终态。页面断开或请求超时先查已有operation，不自动重放写操作。结果unknown_outcome的QQ动作可能已经执行，不再次发送用于确认。

```sh
twintail-host plugins list --data-dir ./data
twintail-host operations list --data-dir ./data
twintail-host config get web-admin --data-dir ./data
```

日志页面只是最多128条脱敏操作journal投影，不是插件stderr或完整审计归档。plugins.installed的verified:false只表示轻量元数据读取，不能作为当前包字节完整性证明。

## 升级前后按这个顺序做

先保存当前版本、grants（已授予权限）、计划及修订，停机备份配置和业务数据，明确恢复点与秘密引用如何在恢复环境解析。不得复制仍在写入的数据并称一致快照。新版本在独立数据根验证后再安排维护窗口；exclusive替换会断开连接并造成事件缺口，不保证零停机。

先升级依赖提供者与业务消费者的兼容组合，依赖顺序以宿主校验为准。配置CAS冲突重新读取并人工合并；startup-only插件采用stop→离线config update→start。重载失败时确认rolled_back/needs_recovery/drain_error，不把completed等同新版上线。旧BotRef在重启/替换后失效。

包恢复与数据恢复分别决策；自动回退旧包不撤销业务数据迁移或外部动作。恢复工具不能重放已发消息。备份/恢复命令及限制见下文。Web不可用时用[CLI救援](../plugin-api/web-admin.md)；宿主不自动重启失败进程，先修复原因再显式start。

## Unix：停机备份与恢复

“冷备份”就是先停下所有写入程序再复制状态。恢复会写到新目录，原目录保留，不做原地覆盖。

需要 Python 3.11+。先确认已停止宿主和全部写入者，再从包含状态工具的源码或解包顶层执行。OLD_DATA须属于当前用户且权限0700；BACKUP/NEW_DATA父目录已存在，目标须不存在或为空0700目录，并与源目录互不嵌套：

```sh
python3 tools/state/state.py snapshot OLD_DATA BACKUP
python3 tools/state/state.py verify BACKUP
python3 tools/state/state.py restore BACKUP NEW_DATA
twintail-host --validate --data-dir NEW_DATA --install-dir ORIGINAL_PACKAGES
```

snapshot使用真实management.sock路径占位，阻止竞争宿主启动；任何现存端点（包括stale socket）或常见run.lock均拒绝，不自动删除。会检查标准plugins/ID/config.lock的OS flock，仅该标准锁路径不备份，业务目录同名文件保留。必须先确认宿主已停，不能将这些检查当作任意外部写入者的通用冻结机制。

备份为私有manifest.json摘要与tree，保留desired/applied和业务文件原字节，不压缩、不读取环境、不运行插件、不转换配置Schema。环境秘密引用保持引用，inline秘密和真实业务数据也仍在备份中；工具不是脱敏或加密工具。不归档ACL/xattr/uid/时间，输出统一文件0600、目录0700；拒链接/硬链接/特殊文件/越界，最多10000条目、单文件64MiB、总1GiB、目录深度32，manifest最多4MiB。拒redacted占位和非法配置envelope，不把desired/applied不一致静默修成一致。

verify只校验结构和摘要，不验证插件Schema、秘密环境可用性或来源。restore只写新/空目标，不覆盖活动数据。恢复后单独注入环境引用，使用原安装根执行host --validate；该命令仅验证包/计划，不验证配置Schema、环境秘密可用或ready，仍需受控启动确认。原applied标记是历史信息，不证明新进程已应用。按插件要求完成业务迁移后再手动启动；Windows named-pipe停机保护未验证，工具明确不支持Windows。

回退需匹配旧host、旧packages与完整旧冷备份，仍恢复到另一个空数据根；保留升级后数据供人工处理，不能将旧配置覆盖新数据称作业务回滚。

详细实现与本地检查入口见[状态工具说明](../../tools/state/README.md)。


## 常见问题与成功判断

| 问题或结果 | 如何判断 |
| --- | --- |
| `plugins list` 为空 | 新数据根且未启用任何插件时属正常，不表示包丢失 |
| 管理命令连接失败 | 确认宿主仍运行且 `--data-dir` 与它相同；不要启动第二个宿主抢占同一数据根 |
| 插件 active 但机器人不回复 | 继续查配置 desired/applied、适配器 connected 和业务权限，最后确认真实收件 |
| 操作 completed | 查 result；重载可能已回退到旧包，不代表新版本上线 |
| 备份报告端点/锁仍存在 | 先确认进程和写入者已停；工具不自动删除 stale socket，不绕过保护直接重试 |
| verify 成功 | 只说明结构和摘要匹配，不证明插件可启动、秘密可用或业务数据已迁移 |
| restore 成功 | 新目录已写入备份；仍需恢复环境引用、离线验证并受控启动确认 |

历史 Linux 本地归档证据记录于 `dist/release-validation.json`，该文件属于当时本地生成报告，不保证每个源码 checkout 都包含它。源码 CI 与实际平台范围见[平台矩阵](platforms.md)。本页没有把未测试平台、真实反向 SnowLuma 或无限期运行列为已验证。
