# 包与插件数据存储

P3 已完成 Linux 本地验收，平台范围见项目记录。安装包与启用计划分开：安装把版本发布到安装根的 `ID/VERSION`；启用计划选择版本和 grants，并触发相应生命周期。仅复制安装成功不会启动进程或授予能力。

本地安装对来源目录做有界检查，将内容复制到安装根同文件系统的 `.install-*` 暂存目录，校验后以 rename 发布。已有 `ID/VERSION` 不覆盖；更新应发布新版本。拒绝链接（含祖先目录链接）、特殊文件与越界路径。当前包限制为 1024 个文件、单文件 64 MiB、总量 256 MiB、目录深度 64。清单入口、Schema 与平台声明仍须通过各自校验。

安装器生成 `checksums.json`，记录全部包文件的 SHA256；来源包若包含 checksums.json，安装前也必须通过摘要检查，安装时重新生成。完整包校验以及启用、重载使用包时验证摘要，损坏会报错，不静默继续加载。WebUI 的 `plugins.installed` 轻量库存与 `plugins.readme` 仅验证元数据和路径，明确返回 `verified: false`，不读取全部可执行文件做哈希；因此出现在库存中不代表内容完整或可信。摘要只能发现当前记录对应的内容变化，不认证作者或下载来源；P3 没有远程注册表或签名信任链。旧包没有摘要时仍可按兼容路径加载；一旦存在摘要，加载时必须验证。安装根保留隐藏的 `.package-store.lock`，使用 OS 文件锁串行修改；进程退出释放锁，不以遗留文件是否存在判断锁定状态。安装完成后不要手工修改清单或入口来适配平台，先修改来源包，再安装成合适版本。

配置与运行数据放在 `data/plugins/ID/`。宿主维护 `config.toml` 期望快照及 `config.applied.toml` 确认修订；SDK initialize 提供的业务 data_dir 为 `data/plugins/ID/data`，由插件自行管理。不要将两个宿主配置文件当作业务文件覆盖。停用和卸载默认保留此目录，重新安装或启用同一 id 可能继续使用旧数据。卸载只能删除未被启用计划引用的指定版本；不会递归清理用户数据。

包目录与数据目录承担不同职责。把可再生成资源放包内，把持久状态放 data_dir；不要依赖源码目录、当前工作目录或 Cargo target 路径。包升级不隐式复制、迁移或回滚业务数据。对于数据库或文件格式升级，插件应提供备份、版本标记和向后兼容策略；恢复旧二进制不等于旧数据仍可读取。

插件独立进程及 capability 校验约束宿主 API，不构成同用户文件系统沙箱。插件仍以宿主用户权限运行，直接子进程资源与派生进程资源的所有权也不同。插件必须保留它创建的任务、监听器与子进程所有权，并在 drain/stop 中关闭和回收。宿主确认直接子进程退出不能证明未追踪的后代已释放端口。

`DATA/operations.json` 管理操作日志保存操作 id、目标、阶段和结果，避免记录完整配置或秘密；最多保留最近 128 条、文件最多 256 KiB。它用于查询中断与恢复状态，不是业务数据备份。重启恢复 pending 操作为 interrupted；对外部服务已经产生的副作用不能据此撤销。

```sh
twintail-host plugins install ./package-directory --data-dir ./data
twintail-host plugins installed --data-dir ./data
twintail-host plugins enable example-plugin 1.0.0 '["events.subscribe"]' --data-dir ./data
twintail-host plugins disable example-plugin --data-dir ./data
twintail-host plugins uninstall example-plugin 1.0.0 --data-dir ./data
```

## 彻底移除插件

`plugins uninstall ID VERSION` 仅卸载版本并保留配置数据。新增独立 `plugins purge ID --confirm ID`：必须先 `plugins disable ID` 并等待终态，然后删除安装根下该 ID 的所有版本与 `data/plugins/ID/` 的配置、已应用快照和业务数据。确认值须与 ID 完全相同，已启用（即使 stopped/failed）仍返回 Conflict；启用中的强依赖者须先处理，不自动级联；未启用包的依赖声明不阻止移除。

```sh
twintail-host plugins disable example-plugin --data-dir ./data
# 查询上一步 operation_id，确认成功后再执行：
twintail-host plugins purge example-plugin --confirm example-plugin --data-dir ./data
```

本机管理与 Web HTTP 管理使用 `plugins.purge`、payload `{ "id": "example-plugin", "confirm_id": "example-plugin" }`；Web 插件须精确获授 `management.write`。WebUI 独立危险按钮要求输入 ID，并提示所有版本和数据将永久删除。没有备份则不支持恢复；环境变量中的秘密源、全局操作日志、其他插件和插件自行写出的目录外文件/远端资源不在删除范围。

操作沿用异步 ACK，通过 operations get 查询实际阶段：`preflighting_purge`、`purging_packages`、`purging_data`。预检拒绝危险链接与路径，但实际删除非原子，IO 或权限变化仍可能造成部分完成；失败不得当作成功，修复原因后可重试，已不存在的目标目录按幂等清理处理。配置 CAS 与移除共享父目录 `.config-locks/` 下按命名空间名 SHA256 命名的稳定锁，作为宿主互斥元数据保留，不包含配置或业务内容；内部 `config.lock` 继续兼容旧锁检查。移除持外部门闩将数据目录退役到 `.purge-ID` 后删除，残留退役目录在重试时一并预检清理。升级时先停止旧版宿主和独立配置写入者，新旧进程并发不属于本次跨版本锁协议保证。正常关闭等待已启动删除收束后再记录终态，强制杀进程仍可能留下部分状态。不要运行插件提供的卸载脚本来替代宿主目录清理。设计见 [ADR 0008](../architecture/0008-plugin-purge.md)。

P3 变更命令返回 running operation，使用 `twintail-host operations get OPERATION_ID --data-dir ./data` 确认终态，再执行依赖前一步结果的操作。enable 的 grants 是 JSON 字符串数组；能力必须由清单声明并获宿主批准。语法已按当前 CLI 源码核对，不能将上面的顺序理解为自动等待整批完成。参阅[清单](manifest.md)、[配置](configuration.md)与[热重载](hot-reload.md)。
