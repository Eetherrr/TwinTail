# 插件候选构建与交付

P6 Linux本地候选完成，用户已授权MIT下GitHub0.1.0发行准备，候选工具仍只构建与验证，不自动上传或发布注册表。发行工具CLI已按源码与负责人合同核对；合同/soak命令已按示例源码核对，最终运行证据单独记录。以下命令提供候选构建路径，不表示所有目标已运行通过。

插件使用版本化IPC，Rust crate版本不等于wire主/次版本。清单的id/version/entrypoint/协议/API范围须与二进制握手一致；清单capability申请与宿主grants分开，默认拒绝。管理插件精确区分management.read/write，业务插件不应申请管理权限。

候选至少包含目标平台二进制、plugin.toml与实际引用的配置Schema，资源只能在包内；不得链接越界，不把本地数据、凭据、host.toml或私有备份混入包。Windows使用.exe入口。版本目录不可原地覆盖。包内摘要只证明被校验文件与摘要一致，不是签名或许可授权。

提交候选前从空目录复现构建/加载，核对显式target与实际运行目标，运行协议合同、超时/取消/背压/旧代消息测试，并在有界soak结束后确认资源与进程退出。默认不使用QQ账号。应用自己spawn的任务须响应取消并join，宿主仅监管直接子进程，禁止派生守护进程。

配置升级写明旧/新config_version、输入拒绝条件、迁移是否可逆及所需备份。在线prepare无副作用；数据破坏迁移必须停机单独执行，不借用exclusive重载宣称自动回滚。参阅[迁移指南](../plugin-api/migration.md)。

兼容报告分别列操作系统、架构、工具链、源码/制品摘要、命令及结果；交叉编译不能冒充目标运行，mock不能冒充真实SnowLuma。项目MIT许可已由用户决定，GitHub发行已授权；公开推送和Release完成状态须另取实际证据。检查制品纳入根LICENSE与相应元数据，第三方依赖许可和再分发义务仍单独核对。

## 从源码生成本地候选

Python3.11+，在同版本完整源码checkout仓库根执行（binary候选包不是完整源码）；使用新的私有输出目录，父目录须已存在。build采用locked release构建，不运行目标制品：

```sh
python3 tools/release/release.py build --target linux-x86_64 --target-dir target
python3 tools/release/release.py stage --target linux-x86_64 --binaries target/x86_64-unknown-linux-gnu/release --output /private/newstage
python3 tools/release/release.py verify /private/newstage
python3 tools/release/release.py archive --stage /private/newstage --output /private/twintail-0.1.0-linux-x86_64.tar.gz
python3 tools/release/release.py verify /private/twintail-0.1.0-linux-x86_64.tar.gz
```

`/private`是示例私有父目录，替换为本机路径。目标映射：linux-x86_64→x86_64-unknown-linux-gnu，linux-aarch64→aarch64-unknown-linux-gnu，windows-x86_64→x86_64-pc-windows-msvc；前两者tar.gz、Windows zip。构建需相应目标工具链与linker，Windows使用MSVC，与历史GNU交叉Clippy证据不同。

stage显式白名单输出bin/twintail-host，以及packages/{adapter-snowluma,web-admin,echo-plugin,mock-adapter}/0.1.0/下binary、plugin.toml、checksums.json和引用的Schema；另含指定docs、schemas、OpenAPI、Cargo与web锁文件、工具链、release.json与SHA256SUMS。Windows自动重写.exe入口和target。不会递归复制配置、data、fixtures或target目录；输入二进制仍须来源可信，白名单不证明二进制没有嵌入秘密。

release.json固定channel=candidate、public_release=false、license_gate=MIT、runtime_verified=false。verify检查候选布局和摘要，不运行二进制、不证明来源，也不代替许可文件与第三方义务核对；即使另有目标运行证据也不手工将此字段改为通过。

## 可信插件合同与合成soak

contract_check会真正启动本地包，可能触发插件定义的启动副作用；仅输入受信任包。路径是包含已构建binary与清单的包目录，按依赖重复传入，grants显式逐个提供，不授予未使用权限：

```sh
cargo run --locked -p twintail-plugin-sdk --example contract_check -- --trusted-package /private/candidate/packages/mock-adapter/0.1.0 --grant bots.register --grant events.publish --grant services.provide --timeout-ms 5000
cargo build --locked -p twintail-plugin-sdk --bin p6-fixture
cargo run --locked -p twintail-plugin-sdk --example bounded_soak -- --fixture "$PWD/target/debug/p6-fixture" --duration-secs 1 --event-count 512 --batch-size 128 --queue-policy drop-newest
```

contract至多32包/128条grant，timeout-ms范围100..30000，默认5000；可用--config ID=JSON_FILE提供ConfigSnapshot（config_version、revision、values），文件最多1MiB，秘密引用经ConfigStore解析，报告不打印值。检查包/计划、初始化active、可选host.health和shutdown；health的unsupported单独报告，不伪造支持。不会主动调用业务RPC，但启动行为本身可有副作用。

soak默认duration1秒、event-count512、batch-size128；范围分别1..300、1..10000、1..256，且ceil(count/batch)≤128，queue-policy仅drop-newest。合成慢消费者过载后按节奏执行echo，报告offered/delivered/dropped/无订阅unrouted、echo计数、generation、health和退出状态，工作总期限duration+30秒。host/SDK event队列32、handlers16，分代刷新限制request id与history使用；fixture复用真实echo，动作实际为send_msg，属于本地mock协议闭环，不代表真实SnowLuma验证。

Linux通过/proc检查已记录PID退出；其他平台只报告shutdown/join，不宣称进程探测。internal_pending_tasks_measured=false，计数是API边界观察，不是内存、任务峰值或长时间生产稳定性证明。最终payload核验版运行报告为dist/p6-soak-report.json：60秒健康输入、6000条send_msg账号/唯一序号核对、24代、49PID退出；总63976毫秒含过载清理。最终workspace源码回归已通过，制品摘要/同输入重复归档验证以dist/release-validation.json为准。

完整参数、默认值、报告字段与最终合同范围见[合同与有界soak工具](../plugin-api/contract-tools.md)，以该专页与实际源码为准。60秒有限负载不证明无限常驻稳定性。
