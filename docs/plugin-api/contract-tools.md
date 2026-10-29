# 插件契约检查与有界负载工具

这些工具运行本地、受信任的原生插件进程。进程隔离不是沙箱，插件的启动和停止回调可能访问文件或网络、产生其自身的副作用。不要检查来源不明的包。工具不连接 QQ 或 SnowLuma，也不证明插件业务行为、操作系统沙箱或无限期常驻能力。

先构建工具和本地负载夹具，编译时间不计入运行报告：

```sh
cargo build -p twintail-plugin-sdk --bin p6-fixture --examples
```

## 包契约检查

```sh
target/debug/examples/contract_check \
  --trusted-package /absolute/package-directory \
  --grant events.subscribe --grant actions.call \
  --timeout-ms 5000
```

`--trusted-package` 可重复列出依赖包，最多 32 个；包需有真实 `plugin.toml` 及可执行 entrypoint。工具复用 core 的包路径、清单、权限、依赖和已安装包摘要检查，再由真实 Supervisor 启动。`--grant` 可重复，最多 128 个，对所有列出的包生效，实际权限取各自 manifest 与 grants 的交集；清单申请不等于授权。缺少必需权限会失败。

工具只调用生命周期协议及宿主到插件方向的 `host.health`，不调用业务方法。握手和启动失败是失败；健康检查返回 `Unsupported` 是可选能力缺失，报告 `optional_unsupported`，不会使检查失败。返回错误健康结构、其他错误或超时会失败。wire 1.2 启动确认包含 SDK start 完成；legacy 只证明旧握手完成。它不支持通过插件自身业务 RPC 证明外部连接已经可用。

需要配置时增加 `--config plugin-id=/absolute/snapshot.json`，内容是 wire `ConfigSnapshot`：

```json
{"config_version":1,"revision":1,"values":{"example":"value"}}
```

文件读取最多 1 MiB；工具用包 schema/config_version 校验并解析环境秘密引用，秘密值和配置内容不进入报告。使用 `{"$secret":"ENV_NAME"}`，由启动工具的环境提供变量。检查使用临时数据目录，不能用来验证已有业务数据的迁移或持久恢复。

stdout 只有一个 JSON 对象，`schema_version` 为 1，包含 `ok`、每包启动状态、健康能力、错误 code、清理状态和时间。失败仅投影安全错误 code，不复制插件错误消息。成功退出 0，契约失败退出 1，非法参数退出 2。`--timeout-ms` 默认 5000，范围 100–30000，分别约束握手、请求和停止；它不是所有包共享的整体期限。停止验证是 Supervisor shutdown 返回和无 active 实例；现有 API 不区分正常 stop 与超时强杀，不测量内部 pending/task 精确数量。

## 合成过载与持续闭环

```sh
target/debug/examples/bounded_soak \
  --fixture target/debug/p6-fixture \
  --duration-secs 1 --event-count 64 --batch-size 32 \
  --queue-policy drop-newest
```

Windows 可执行路径需要 `.exe`；平台运行结果以兼容矩阵为准。`--fixture` 必须是本仓库构建的受信任 `p6-fixture`，工具不验证该二进制的来源。它复用真实 mock-adapter、echo-plugin 及一个故意停止读取管道的故障订阅者。没有 QQ 账号、外部服务器或随机数据依赖。

先向慢订阅者发布固定 128 个 128 KiB 事件，确认其队列满时出现 `dropped > 0`。慢实例可能因管道写超时被独立退休；随后无订阅者的事件单独统计 `unrouted_after_slow_retirement`，不伪装为队列丢弃。发布者仍返回结果，故障订阅者停止后健康 mock→echo→action 路由继续服务。健康阶段每个事件带唯一序号，验证所有返回动作的 `send_msg`、账号、消息段序号集合，防止只用计数掩盖重复或遗漏。

健康工作按目标 duration 分布输入，每批核对动作并执行健康请求；不会在完成全部工作后只 sleep 来充数。每批正常停止 echo/mock 并启动新代次，代次切换显式出现在报告。该实验不声称一个无限运行的连接永不耗尽累计预算。

参数和边界：

| 参数或资源 | 默认 / 上限 |
| --- | --- |
| duration-secs | 1 / 1–300 秒的目标工作时长 |
| event-count | 512 / 1–10000 个健康事件，另加固定 128 过载事件 |
| batch-size | 128 / 1–256，`ceil(event-count / batch-size) <= 128` |
| queue-policy | 只支持 drop-newest；其他值退出 2 |
| 总工作 deadline | duration + 30 秒，另有 shutdown 清理 |
| 宿主 event lane | 每实例 32 |
| SDK event queue / handlers | 32 / 16 |
| 连接累计 request ID 预算 | 4096，每个新代次重新计数 |
| mock 动作历史 | 1024；实验每代最多 batch-size <=256 条 |

报告区分 offered、delivered 入队次数、dropped、无订阅者、验证动作、代次、健康检查及观察到的历史峰值。入队成功不代表业务完成。Linux 通过夹具写出的直接子进程 PID 和 `/proc` 验证它们在停止后消失；其他平台只验证 Supervisor 的停止返回，报告明确标注 PID 测量不可用。内部 pending、任务数、RSS、整个进程树内存和泄漏趋势没有精确量测，不能据本报告宣称硬内存上限或完整性能认证。

60 秒验收命令：

```sh
target/debug/examples/bounded_soak \
  --fixture target/debug/p6-fixture \
  --duration-secs 60 --event-count 6000 --batch-size 256 \
  --queue-policy drop-newest > dist/p6-soak-report.json
```

`dist/` 需先创建。输出 JSON 不含配置、秘密值或原始业务日志；它是当前机器当前构建的实验记录，运行时长和 drop/unrouted 分配随机器速度变化。`cargo test -p twintail-plugin-sdk --test p6` 自动运行短测、可选健康兼容、启动失败与参数边界；无需在测试中嵌套启动 Cargo。
