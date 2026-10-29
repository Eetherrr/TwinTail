# ADR 0002：P1 插件宿主与 IPC v1

- 状态：已实施；Linux 本地验收通过，Windows 运行待验证
- 日期：2026-10-28
- 范围：P1；替代早期设计中未冻结的线格式。

## 决策

独立进程的 stdin/stdout 使用 4 字节大端长度和 JSON。`protocol_version` 为 `{major, minor}`，`kind` 为 request/response/event/control。响应统一为 `outcome: {status: "ok"|"err", value: ...}`，由 Rust 枚举排除同时成功和失败。P1 不提供 trace_id，允许未知可选字段；只有主版本必须匹配，协商最小次版本。所有请求 ID 限长、非空，连接内不复用；超时或非法响应终止该代连接，防止迟到结果误投递。

握手顺序：插件 hello → 宿主 negotiate → 宿主 initialize → 插件 ready → 宿主 activate。宿主将管道与 manifest 身份绑定，验证 hello 身份、版本和必需能力；插件自报身份不参与授权。initialize 提供数据目录、授权、代次、实例标识及 config_revision=0（P1 不加载插件配置）。只有 ready 后的实例才能调用 host.health，并且仍需 `host.health` 能力。P1 不启动事件/动作/服务路由，未知方法返回 Unsupported。

控制帧与请求响应使用独立有界队列；宿主调用在途表有上限、固定 deadline。任何读帧、队列、写入和关闭路径有上限；超大帧分配前拒绝。stderr 按字节消费且不原样写日志，避免泄露凭据、终端转义及无换行日志占用内存。P1 不提供日志查看 API。

插件清单冻结最小子集：id/name/version/protocol_version/host_api/entrypoint/args/capabilities/required_capabilities/dependencies。未实现的平台选择、可选依赖、reload_policy、配置声明等字段不可用于声称功能生效。强依赖版本验证和拓扑排序，逆序停止；停止仍被依赖的实例默认 Conflict。依赖故障立即撤销可用性，并停止传递依赖者，不做无限自动重启；P1 重启预算为 0，只允许管理员显式 start（P3 再引入有界自动恢复）。每次 start 增加 generation，失败实例不会自动占用新代次。

管理请求经独立本机端点进入有界命令队列，不复用插件身份。Unix 使用仅当前有效用户可访问的 0700 目录和 0600 socket，并验证双向 peer credentials。已有 socket 拒绝覆盖；异常退出残留需确认进程结束后手动删除。Windows 使用具有当前用户 ACL 的命名管道；若安全 ACL 尚不可用必须 fail closed 并如实标记平台验收限制，不回退到默认不受限管道。

进程隔离不是沙箱。P1 只接收受信任包，采用绝对可执行路径和无 shell 启动；清除继承环境中的秘密，保留必要平台环境；包应由操作者保护，路径验证不替代对并发文件替换的防御。P1 监管直接子进程、限时停止并 wait 回收；插件不得派生脱离监管的子进程。完整进程树管理是 P3 热重载前置工作，不宣称当前已达到热重载进程树保证。

## 兼容与迁移

P0 `--check` 保留为不读配置、不写盘的引导检查；新增 `--validate` 离线验证 host.toml 与安装包，正常 run 加载 host.toml（不存在则空计划）。P1 清单以 schema 与 docs/plugin-api 为准。后续新增声明字段遵循协议 minor/host API 兼容范围；不支持的方法返回明确错误，不以空成功响应掩盖缺失功能。

## 验证

独立测试插件覆盖启动/停止、版本错误、坏帧/超大帧、握手和请求超时、越权、崩溃和依赖故障。生成的 schema 与源码通过 CI --check 校验。具体检查结果与平台限制在兼容性记录登记。

## 生命周期取消与 Windows 管理补充（2026-10-29）

启动任务登记在实例槽中；取消 start 等待不丢失任务，shutdown 等待有界握手完成并停止/回收直接子进程。停止任务的 JoinHandle 跨 await 留在槽中；管道 reader/writer/stderr 使用 abort-on-drop 保护。正常退出应调用 shutdown；直接 Drop 仅提供取消与 kill-on-drop，不承诺同步 wait。管理 dispatch 可由退出信号中断；启动期间每 25ms 协调其他实例故障。

Windows 使用精确固定 interprocess 2.4.7 的首实例保护、禁止远程、当前进程用户 SID 专用 protected DACL，并双向查询 peer process token SID。连接名由 SID 与规范化数据路径的 SHA-256 生成。管道传输在完整响应后使用单字节 ACK 与 EOF，避免服务端提前关闭丢弃响应；取消路径 assume_flushed 后立即关闭，避免库的无限后台 flush。Unix 与 Windows 网络阶段期限均 5 秒，等待生命周期响应上限 300 秒；超时不等于操作撤销，先查询状态再重试。管理请求串行执行，尚无异步 operation id；大量排队请求仍可能超时，P3 将引入操作记录。
