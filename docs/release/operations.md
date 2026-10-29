# 用户运维与候选升级

P6 Linux本地候选完成，本文维护可操作的运维边界；发行与状态工具命令已按实际接口核对，最终源码回归已通过，制品归档验证见dist/release-validation.json。用户已授权MIT下GitHub0.1.0发行准备；本文不宣称公开推送或Release已完成，也不提供未测试平台支持承诺。

## 首次部署

使用私有数据根与独立安装根，运行身份拥有最小必要的文件/网络权限。安装包只存版本制品，配置、desired/applied修订、操作记录和业务数据留在数据根。先检查包目标平台、来源和摘要；摘要用于损坏检测，不是签名或来源认证。只运行受信任插件，独立进程不是沙箱。

从源码打包和首次配置的已验收入口：[Web管理](../plugin-api/web-admin.md)、[正向适配器](../plugin-api/adapters.md)、[无QQ示例](../plugin-api/quickstart.md)。Windows包须使用.exe入口，安装后不原地修改清单/二进制。宿主启动前注入环境秘密引用，浏览器不写入明文token。无配置的空计划不是错误。

## 日常检查

同时判断进程active、配置desired/applied、外部连接connected与业务响应；单项成功不能替代其他层。写操作先接纳再后台执行，保存operation id并查询终态。页面断开或请求超时先查已有operation，不自动重放写操作。结果unknown_outcome的QQ动作可能已经执行，不再次发送用于确认。

```sh
twintail-host plugins list --data-dir ./data
twintail-host operations list --data-dir ./data
twintail-host config get web-admin --data-dir ./data
```

日志页面只是最多128条脱敏操作journal投影，不是插件stderr或完整审计归档。plugins.installed的verified:false只表示轻量元数据读取，不能作为当前包字节完整性证明。

## 升级与恢复

先保存当前版本、grants、计划及修订，停机备份配置和业务数据，明确恢复点与秘密引用如何在恢复环境解析。不得复制仍在写入的数据并称一致快照。候选在独立数据根验证后再安排维护窗口；exclusive替换会断开连接并造成事件缺口，不保证零停机。

先升级依赖提供者与业务消费者的兼容组合，依赖顺序以宿主校验为准。配置CAS冲突重新读取并人工合并；startup-only插件采用stop→离线config update→start。重载失败时确认rolled_back/needs_recovery/drain_error，不把completed等同新版上线。旧BotRef在重启/替换后失效。

包恢复与数据恢复分别决策；自动回退旧包不撤销业务数据迁移或外部动作。恢复工具不能重放已发消息。备份/恢复命令及限制见下文。Web不可用时用[CLI救援](../plugin-api/web-admin.md)；宿主不自动重启失败进程，先修复原因再显式start。

## Unix冷备份与恢复

Python3.11+，停止宿主及所有写入者后执行。OLD_DATA须属于当前用户且权限0700；BACKUP/NEW_DATA父目录已存在，目标须不存在或为空0700目录，并与源目录互不嵌套：

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
