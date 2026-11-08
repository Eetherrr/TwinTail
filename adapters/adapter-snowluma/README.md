# adapter-snowluma

## 用途与能力

独立进程的 OneBot v11 Universal WebSocket 适配器，连接独立部署的 SnowLuma；不负责 QQ 登录或 SnowLuma 自身管理。支持正向 `forward_ws`（默认）和仅数字 loopback 监听的反向 `reverse_ws`，不支持 split WS 或 HTTP 传输。

必须由宿主授予 `bots.register`、`events.publish`、`services.provide`。注册配置账号的 BotRef，发布 `message`、`notice`、`request`、`meta_event` 及合法未知 post_type 事件，保留原始 JSON/消息段。业务动作通过 BotRef 路由；`onebot.v11@1.0.0` 服务提供 `connection.status`，不是通用动作透传服务。

## 配置

清单版本 `0.1.0`，`config_version = 1`，配置以随包 `config.schema.json` 和运行时语义校验为准。首次启用前，在私有数据根的 `plugins/adapter-snowluma/config.toml` 预置：

```toml
config_version = 1
revision = 1

[values]
transport = "forward_ws"
url = "ws://127.0.0.1:6700/"
self_id = "10001"

[values.access_token]
"$secret" = "SNOWLUMA_ACCESS_TOKEN"
```

地址与账号替换为实际部署值。宿主启动前注入同名环境秘密；无鉴权时删除整个 `values.access_token` 表。token 只通过 Bearer 传送，不放入 URL。反向模式省略 `url`，设置 `transport = "reverse_ws"`、`reverse_bind = "127.0.0.1:17281"`、`reverse_path = "/onebot/v11"`；SnowLuma 使用精确账号 `X-Self-ID`、`X-Client-Role: Universal` 和匹配的 Bearer 连接该地址。

常用默认值：动作/连接超时各 3000ms，重连退避 100–10000ms，ping 15000ms、pong 超时 10000ms，pending/事件队列各 64，帧 1MiB。`self_id` 为十进制字符串；限制范围见 Schema。

## 使用与重载

构建或使用发行包中的可执行文件、清单、Schema、README 与摘要；源码目录本身不是可执行包。宿主运行后，通过本机 CLI 安装、查询操作完成，再启用：

```sh
twintail-host plugins install PACKAGE_DIR --data-dir DATA_DIR
twintail-host plugins enable adapter-snowluma 0.1.0 '["bots.register","events.publish","services.provide"]' --data-dir DATA_DIR
twintail-host plugins call adapter-snowluma connection.status '{}' --data-dir DATA_DIR
```

install/enable 返回的 operation_id 须用 `operations get OPERATION_ID` 等待终态，再进行下一步。IPC active 不等于 WS connected；查看 `connected`、`phase`、`last_error` 等状态。正向连接支持断线重连，但动作断线/超时可能为结果未知，不自动重发副作用动作。

所有配置为启动期配置；现有实例按 stop → 带 expected_revision 的 config update → start 操作，每步等待完成。包替换采用 `exclusive` 重载，断开并重建连接，可能丢失事件，不承诺零停机或业务数据回滚。正向真实部署已有指定范围验证；反向真实 SnowLuma 与其他平台/动作范围仍须单独验证。
