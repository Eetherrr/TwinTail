# 权限：申请能力不等于已经获得

能力（capability）是宿主 API 的使用许可，例如订阅事件或发送动作。插件作者在清单中说明需要什么，管理员在启用计划中决定授予什么。本页用于新插件配置授权或排查 `permission_denied`。

前置知识：先看[清单](manifest.md)中的 `capabilities`、`required_capabilities` 和 `grants`。required 表示“没有就不要启动”；非 required 的已申请能力允许不授予，插件应处理相应调用失败。

## 一次调用如何检查权限

SDK 的 `Context` 不是超级用户通道。`context.initialize().granted_capabilities` 可查看启动时实际授予的 `Vec<String>`；调用仍由宿主按管道身份逐次校验，失败返回 `RpcError { code: ErrorCode::PermissionDenied, message }`。准确字段见 [Initialize](../../crates/twintail-protocol/src/lib.rs)，各方法签名见[SDK 参考](sdk-reference.md#context公共方法)。

能力默认拒绝，实际授予是清单 capabilities 与 host.toml grants 的精确交集，去重排序。没有前缀、通配符或隐含授权。required_capabilities 未获授予会阻止加载；hello 的必需能力集合必须与清单一致。未知能力名返回 unsupported。

| 能力名 | 当前语义 |
| --- | --- |
| `host.health` | 已有同名宿主方法，需要显式申请并授予 |
| `host.read` | 已识别的能力词汇；当前没有对应的已实现读取方法 |
| `bots.register` | 注册调用者实例下的账号；不能指定其他 adapter instance |
| `events.subscribe` | 替换精确 event_type 订阅集合 |
| `events.publish` | 仅发布调用者已注册账号的事件 |
| `actions.call` | 调用 BotRef 指定适配器的动作；P2 为粗粒度权限 |
| `services.provide` | `services.register` 注册命名和版本化服务 |
| `services.call` | 按显式 provider、service 与 SemVer 范围调用服务 |
| `storage.private` | 预留词汇；P1 提供初始化数据目录，无存储 RPC |
| `management.read`、`management.write` | 独立管理桥接的精确只读/写权限，互不隐含；`plugins.readme {id,version}` 仅要求 read；`plugins.purge {id,confirm_id}` 要求 write，必须先停用且精确确认 ID，永久清除全部版本及该插件配置/数据，不清外部资源或审计记录；方法白名单见 [Web 管理契约](web-admin.md) |

## 授权示例与身份

例如 capabilities 和 required_capabilities 都含 `host.health` 时，host.toml 对应条目的 grants 必须含该精确名称。只在 grants 写入、清单未申请的能力不会授予。

宿主按实际启动管道绑定调用者，不以消息中的自报 plugin_id 授权。本机 CLI 管理使用独立 OS 用户身份校验，不由插件 hello 或 capabilities 提升为管理主体。

## 宿主权限约束到哪里

**框架权限不是操作系统沙箱。** 同用户插件可以直接访问该用户的文件、网络或本机管理端点；私有目录命名空间、精简环境和能力交集不能阻止恶意本地代码。P1 仅适用于操作者信任的本地插件，没有不可信包隔离或来源签名认证。详见[安全边界](../architecture/security.md)。

P2 的动作权限不按动作名称、机器人或参数进一步限制；服务调用权限不按提供者/方法进一步限制。BotRef、版本解析和注册归属校验不会替代更细粒度的授权。只有获得粗粒度调用能力的受信任插件可以使用相应路由。
