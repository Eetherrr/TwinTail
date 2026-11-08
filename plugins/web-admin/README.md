# web-admin

## 用途与能力

独立进程的本机 Web 管理插件，提供内嵌前端、认证 HTTP API 和宿主管理桥接。浏览器可查看运行状态、插件、配置、操作及有界诊断；写操作由宿主权限控制，不直接写配置文件，也不管理 SnowLuma 的 QQ 登录。

必须授予 `management.read`；需要启停、配置更新和重载等写操作时再授予 `management.write`。浏览器登录不增加宿主能力。仅支持数字 loopback 监听；没有远程监听、TLS、管理 WebSocket 或原始 stderr 日志订阅。

## 配置

清单版本 `0.1.0`，`config_version = 1`。首次启用前，在私有数据根的 `plugins/web-admin/config.toml` 预置：

```toml
config_version = 1
revision = 1

[values]
bind = "127.0.0.1:17280"

[values.admin_token]
"$secret" = "TWINTAIL_ADMIN_TOKEN"
```

宿主启动前通过环境注入同名随机秘密，32–256 个可打印非空格 ASCII 字符。存储配置使用环境秘密引用，勿保存明文 token。浏览器配置读取保留引用，不回传已解析秘密；更换 token 需重启，会使所有会话失效。

随包 `config.schema.json` 定义完整限制。默认会话 TTL 3600 秒、最多 32 会话、16 并发请求、64 连接、请求期限 10000ms、请求体 65536 字节。精确 Host/Origin、CSRF、HttpOnly/SameSite 会话及固定 TTL 校验由插件执行。

## 使用与重载

发行包已嵌入前端，无需运行 npm。宿主运行后安装包含可执行文件、清单、Schema、README 与摘要的包，再启用：

```sh
twintail-host plugins install PACKAGE_DIR --data-dir DATA_DIR
twintail-host plugins enable web-admin 0.1.0 '["management.read","management.write"]' --data-dir DATA_DIR
```

每次操作返回 operation_id，用 `operations get OPERATION_ID` 等待终态后进行下一步。浏览器打开 `http://127.0.0.1:17280`，使用注入的 token 登录；若仅授予 `management.read`，管理写调用仍被宿主阻止。

所有配置为启动期配置；更新按 stop → 带 expected_revision 的 config update → start，每步等待完成。包替换采用 `exclusive` 重载，监听端口与会话重建，浏览器请求可能中断；web-admin 自身停用或失败时使用本机 CLI 恢复。插件无业务数据迁移机制，宿主配置和数据目录独立于版本包，重载不回滚数据。
