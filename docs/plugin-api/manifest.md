# 清单、安装与启用计划

P3 本地包与启用管理已通过 Linux 本地验收，平台范围见项目记录。宿主从 `DATA/host.toml` 读取显式启用包。文件不存在表示空计划；存在时校验整个计划，失败不返回部分计划。`--validate` 离线读取配置和包，不执行插件、不创建运行目录。正常启动会创建私有数据根并启动计划中的进程。

包内的 `plugin.toml` 示例：

```toml
id = "fixture"
name = "P1 protocol test fixture"
version = "0.1.0"
protocol_version = 1
host_api = "^0.1.0"
entrypoint = "p1-fixture"
args = ["normal", "fixture"]
capabilities = []
required_capabilities = []
config_version = 1
reload_policy = "exclusive"
# config_schema = "config.schema.json"
# target = "linux-x86_64"

[dependencies]
# provider = "^0.1.0"
```

这是[测试 fixture 流程](troubleshooting.md)使用的静态清单，不是 SDK 示例。`id`、`name`、`version`、`protocol_version`、`host_api`、`entrypoint` 必填；args、capabilities、required_capabilities、dependencies 缺省为空。未知可选字段忽略。P3 新增 config_schema、config_version、reload_policy 与 target；没有签名或许可证认证。未知字段忽略不代表宿主实现其语义。

- id 首字符小写 ASCII 字母，其余限小写字母、数字、`.`、`_`、`-`；禁止末尾点及 Windows 保留设备名（含扩展形式）。
- name 非空；version 是 SemVer；protocol_version 为整数 1；host_api 是匹配宿主 API `0.1.0` 的 SemVer 范围。
- entrypoint 是包内相对文件路径；args 为原样传递的参数数组，不经 shell。
- required_capabilities 必须同时出现在 capabilities 并获宿主显式授予；能力细节见[权限](permissions.md)。
- dependencies 是提供者 id 到 SemVer 范围的映射，全部为强依赖，无可选依赖。

对应的 `DATA/host.toml`：

```toml
[[plugins]]
path = "fixture"
grants = []
```

path 相对 `--install-dir`，与配置文件位置无关。安装根、包根、manifest 和入口均规范化；拒绝绝对路径、父路径、Windows 驱动器/反斜线语法及越出相关根的符号链接。入口必须是文件，执行权限及平台二进制兼容性在 spawn 时确认。包应在校验到执行期间保持不可变；规范化检查不能消除同用户替换文件的竞争。

host.toml 上限 1 MiB、插件数上限 128、每份 plugin.toml 上限 64 KiB。重复 id、缺失/版本不兼容依赖和环阻止整个计划。拓扑启动时可用节点按 id 排序，停止顺序相反。运行时提供者失效会停止传递依赖者；手动停止仍有活动依赖者的提供者返回 conflict。

P3 字段及安装语义：

- `config_schema` 缺省为空，指定时为包内相对文件路径且必须存在；内容按离线 Draft 2020-12 子集校验，参阅[配置](configuration.md)。
- `config_version` 缺省为 1，是插件配置格式版本，与配置修订号独立。
- `reload_policy` 缺省为 `exclusive`；`parallel_safe` 返回 unsupported，P3 不自动降级为排他模式。
- `target` 缺省为空，旧包继续兼容但不声称已认证平台。支持 `linux-x86_64`、`windows-x86_64`、`linux-aarch64`；未知目标返回 unsupported，与当前 OS/架构不符返回 incompatible_version。

`plugins install PATH` 安装到 `ID/VERSION` 并生成 checksums.json；安装成功不代表启用。`plugins enable ID VERSION GRANTS_JSON` 选择版本与授权；变更异步操作须查询终态。旧的相对 path 示例展示离线计划字段，管理生成的版本路径以实际宿主实现为准。包不可原地覆盖，Windows 应在安装前将来源清单 entrypoint 与可执行文件同时设为 `.exe`。参阅[存储](storage.md)和[迁移](migration.md)。
