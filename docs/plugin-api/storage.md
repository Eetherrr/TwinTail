# 包与插件数据存储

P3 已完成 Linux 本地验收，平台范围见项目记录。安装包与启用计划分开：安装把版本发布到安装根的 `ID/VERSION`；启用计划选择版本和 grants，并触发相应生命周期。仅复制安装成功不会启动进程或授予能力。

本地安装对来源目录做有界检查，将内容复制到安装根同文件系统的 `.install-*` 暂存目录，校验后以 rename 发布。已有 `ID/VERSION` 不覆盖；更新应发布新版本。拒绝链接（含祖先目录链接）、特殊文件与越界路径。当前包限制为 1024 个文件、单文件 64 MiB、总量 256 MiB、目录深度 64。清单入口、Schema 与平台声明仍须通过各自校验。

安装器生成 `checksums.json`，记录全部包文件的 SHA256；来源包若包含 checksums.json，安装前也必须通过摘要检查，安装时重新生成。列举和使用已安装包时验证摘要，损坏会报错，不静默继续加载。摘要只能发现当前记录对应的内容变化，不认证作者或下载来源；P3 没有远程注册表或签名信任链。旧包没有摘要时仍可按兼容路径加载；一旦存在摘要，加载时必须验证。安装根保留隐藏的 `.package-store.lock`，使用 OS 文件锁串行修改；进程退出释放锁，不以遗留文件是否存在判断锁定状态。安装完成后不要手工修改清单或入口来适配平台，先修改来源包，再安装成合适版本。

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

P3 变更命令返回 running operation，使用 `twintail-host operations get OPERATION_ID --data-dir ./data` 确认终态，再执行依赖前一步结果的操作。enable 的 grants 是 JSON 字符串数组；能力必须由清单声明并获宿主批准。语法已按当前 CLI 源码核对，不能将上面的顺序理解为自动等待整批完成。参阅[清单](manifest.md)、[配置](configuration.md)与[热重载](hot-reload.md)。
