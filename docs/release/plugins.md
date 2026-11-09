# 从源码构建程序包并交付插件

本页面向想自己构建发行归档、或把插件交给别人使用的人。你会先得到一个可以检查的本地包，再用独立测试确认它能启动和退出。**构建工具不上传、不打 tag，也不执行 `cargo publish`。** 使用 GitHub 自动化准备 Release 草稿请看[自动化发行](automation.md)。

## 先分清四个步骤

| 步骤 | 做什么 | 成功不代表什么 |
| --- | --- | --- |
| `build` 构建 | 把源码编译成指定平台程序 | 不运行目标程序，不证明其业务可用 |
| `stage` 整理 | 从白名单收集程序、清单、许可和文档到新目录 | 不证明二进制来源可信或没有嵌入秘密 |
| `archive` 压缩 | 生成 `.tar.gz` 或 `.zip` 下载包 | 不上传、不公开发行 |
| `verify` 校验 | 检查布局、文件类型、目标架构和摘要 | 不执行插件，不是签名或业务验证 |

工具保留 `candidate`（待复查包）标记，避免把打包成功说成公开发行或运行认证。下面先给操作，再给插件作者必须遵守的接口约定。

## 构建前准备

1. 使用同版本完整源码，在仓库根执行。预编译包不是完整源码，不能直接拿其中不完整工程运行 Cargo。
2. 安装仓库固定 Rust 1.99.1、相应 target 与链接器，以及 **Python 3.11+**。构建 WebUI 已提交静态资源不需 Node；若改了前端，先按 Web 管理指南重建资源。
3. 准备只有当前用户可写的私有父目录。下面 `/private` 只是占位路径，先换成你机器上已存在的目录；输出 stage 和归档必须是尚不存在的新路径。
4. 确认要运行的目标和交付对象。Linux x86_64、Linux aarch64 与 Windows x86_64 MSVC 是不同程序，不相互通用。

## 第 1 步：构建、整理并校验归档

Python3.11+，在同版本完整源码checkout仓库根执行（预编译包不是完整源码）；使用新的私有输出目录，父目录须已存在。build采用locked release构建，不运行目标制品：

```sh
python3 tools/release/release.py build --target linux-x86_64 --target-dir target
python3 tools/release/release.py stage --target linux-x86_64 --binaries target/x86_64-unknown-linux-gnu/release --output /private/newstage
python3 tools/release/release.py verify /private/newstage
python3 tools/release/release.py archive --stage /private/newstage --output /private/twintail-0.1.0-linux-x86_64.tar.gz
python3 tools/release/release.py verify /private/twintail-0.1.0-linux-x86_64.tar.gz
```

`/private`是示例私有父目录，替换为本机路径。目标映射：linux-x86_64→x86_64-unknown-linux-gnu，linux-aarch64→aarch64-unknown-linux-gnu，windows-x86_64→x86_64-pc-windows-msvc；前两者tar.gz、Windows zip。构建需相应目标工具链与linker，Windows使用MSVC，与历史GNU交叉Clippy证据不同。

stage 按白名单输出 `bin/twintail-host`，以及 `packages/{adapter-snowluma,web-admin,echo-plugin,mock-adapter}/0.1.0/` 下的二进制、`plugin.toml`、`README.md`、`checksums.json` 和引用的 Schema；另含指定文档、schemas、OpenAPI、Cargo/Web 锁文件、工具链、MIT LICENSE、第三方许可材料、`release.json` 与 `SHA256SUMS`。Windows自动重写.exe入口和target。不会递归复制配置、data、fixtures或target目录；输入二进制仍须来源可信，白名单不证明二进制没有嵌入秘密。

release.json固定channel=candidate、public_release=false、license_gate=MIT、runtime_verified=false。verify检查候选布局和摘要，不运行二进制、不证明来源，也不代替许可文件与第三方义务核对；即使另有目标运行证据也不手工将此字段改为通过。

## 第 2 步：真正启动插件做受控检查

“合同检查”是确认插件与宿主能按约定握手、初始化和退出；“有界负载测试（soak）”是在明确时长和工作量内观察路由与清理，不是无限期运行认证。下面命令会启动代码，先核对受信任包与授予权限。

contract_check会真正启动本地包，可能触发插件定义的启动副作用；仅输入受信任包。路径是包含已构建binary与清单的包目录，按依赖重复传入，grants显式逐个提供，不授予未使用权限：

```sh
cargo run --locked -p twintail-plugin-sdk --example contract_check -- --trusted-package /private/newstage/packages/mock-adapter/0.1.0 --grant bots.register --grant events.publish --grant services.provide --timeout-ms 5000
cargo build --locked -p twintail-plugin-sdk --bin p6-fixture
cargo run --locked -p twintail-plugin-sdk --example bounded_soak -- --fixture "$PWD/target/debug/p6-fixture" --duration-secs 1 --event-count 512 --batch-size 128 --queue-policy drop-newest
```

contract至多32包/128条grant，timeout-ms范围100..30000，默认5000；可用--config ID=JSON_FILE提供ConfigSnapshot（config_version、revision、values），文件最多1MiB，秘密引用经ConfigStore解析，报告不打印值。检查包/计划、初始化active、可选host.health和shutdown；health的unsupported单独报告，不伪造支持。不会主动调用业务RPC，但启动行为本身可有副作用。

soak默认duration1秒、event-count512、batch-size128；范围分别1..300、1..10000、1..256，且ceil(count/batch)≤128，queue-policy仅drop-newest。合成慢消费者过载后按节奏执行echo，报告offered/delivered/dropped/无订阅unrouted、echo计数、generation、health和退出状态，工作总期限duration+30秒。host/SDK event队列32、handlers16，分代刷新限制request id与history使用；fixture复用真实echo，动作实际为send_msg，属于本地mock协议闭环，不代表真实SnowLuma验证。

Linux通过/proc检查已记录PID退出；其他平台只报告shutdown/join，不宣称进程探测。internal_pending_tasks_measured=false，计数是API边界观察，不是内存、任务峰值或长时间生产稳定性证明。历史 payload 核验版运行报告为 `dist/p6-soak-report.json`：60秒健康输入、6000条send_msg账号/唯一序号核对、24代、49PID退出；总63976毫秒含过载清理。该次 workspace 源码回归通过，制品摘要/同输入重复归档记录在 `dist/release-validation.json`，这些是历史本地生成报告，不保证存在于每个源码 checkout。

完整参数、默认值、报告字段与最终合同范围见[合同与有界soak工具](../plugin-api/contract-tools.md)，以该专页与实际源码为准。60秒有限负载不证明无限常驻稳定性。

## 插件作者的交付约定

插件使用版本化IPC，Rust crate版本不等于wire主/次版本。清单的id/version/entrypoint/协议/API范围须与二进制握手一致；清单capability申请与宿主grants分开，默认拒绝。管理插件精确区分management.read/write，业务插件不应申请管理权限。

交付包至少包含目标平台二进制、plugin.toml与实际引用的配置Schema，资源只能在包内；不得链接越界，不把本地数据、凭据、host.toml或私有备份混入包。Windows使用.exe入口。版本目录不可原地覆盖。包内摘要只证明被校验文件与摘要一致，不是签名或许可授权。

交付前从空目录复现构建/加载，核对显式target与实际运行目标，运行协议合同、超时/取消/背压/旧代消息测试，并在有界soak结束后确认资源与进程退出。默认不使用QQ账号。应用自己spawn的任务须响应取消并join，宿主仅监管直接子进程，禁止派生守护进程。

配置升级写明旧/新config_version、输入拒绝条件、迁移是否可逆及所需备份。在线prepare无副作用；数据破坏迁移必须停机单独执行，不借用exclusive重载宣称自动回滚。参阅[迁移指南](../plugin-api/migration.md)。

兼容报告分别列操作系统、架构、工具链、源码/制品摘要、命令及结果；交叉编译不能冒充目标运行，mock不能冒充真实SnowLuma。项目使用 MIT，v0.1.0 已公开发行；此处新构建的包是否已公开，需要独立发布记录。检查制品纳入根LICENSE与相应元数据，第三方依赖许可和再分发义务仍单独核对。



## 出错后怎么查

- 构建失败：先看 Cargo 输出，核对固定工具链、target 和链接器；不要改成另一目标后沿用原平台标签。
- stage/verify 失败：检查是否误用了已有输出目录，输入是否为五个正确平台二进制，许可/白名单文件是否齐全；不要修改摘要掩盖内容不一致。
- contract 报失败：核对清单入口、依赖、grants 和必填配置，按报告区分启动失败与健康检查不支持。
- 测试通过后：保留源码提交、锁文件、工具链、平台、命令、报告和归档 SHA256。发送给用户时附用途、前置环境、配置、升级/恢复说明，以及未验证范围；不要只发一个可执行文件。
