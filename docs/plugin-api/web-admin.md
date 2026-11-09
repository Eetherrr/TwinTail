# 用浏览器管理 TwinTail：安装、操作与恢复

安装 `web-admin` 插件后，你可以在本机浏览器查看插件、安装本地包、启停或重载插件、修改配置和查看操作结果。默认打开 **http://127.0.0.1:17280/**；它不是你正在阅读的文档网站。

## 开始前先分清入口

- **本文档网站**：说明书，没有机器人管理功能，也没有管理登录凭据。
- **TwinTail WebUI**：`web-admin` 插件提供的本机管理台，管理 TwinTail 宿主与插件。
- **SnowLuma 管理页**：独立部署的 SnowLuma 提供，负责它自身的 QQ 登录与账号运维。TwinTail WebUI 不替代它。
- **CLI（命令行）**：在终端调用 `twintail-host` 的管理命令。WebUI 故障时仍能连接同一个宿主恢复服务。

先按下面步骤装好管理插件，再在浏览器登录。后文的 HTTP/IPC 表格是开发接口参考，第一次使用可先跳过。

## 入口与凭据

web-admin 是独立插件，通过宿主管理 IPC 操作插件，不直接修改配置文件，也不连接 SnowLuma 的登录或私有管理接口。默认入口为 `http://127.0.0.1:17280/`，当前只允许 loopback 地址。这里的 localhost 指 IP `127.0.0.1`；浏览器访问应使用配置的精确入口，不能把绑定地址改为任意网卡来提供远程服务。

管理登录 token 使用宿主环境秘密引用，解析后至少32字节；真实 token 不写进清单、示例、URL 或操作日志。宿主须在启动前取得相应环境变量。浏览器登录后使用 HttpOnly、SameSite=Strict 会话 cookie；管理请求另须 CSRF 校验。登录 token 不放入 localStorage，不向 SnowLuma 或其他插件页面发送。会话固定有效期默认3600秒，不随操作续期；插件重启后旧会话失效。HTTP只服务loopback，不支持TLS终止或可信代理转发配置，cookie在该HTTP模式不带Secure。

清单申请管理权限仍需宿主显式授予：读请求要求 exact `management.read`，写请求要求 exact `management.write`。POST 管理桥接不因 HTTP 方法统一而把读操作视为写操作，也不把 `management.write` 解释为隐含读权限。普通 feature 插件不应获得这些能力。web-admin 拥有安装和配置能力，必须作为受信任本机管理程序运行；进程隔离不是沙箱。

## 安装与日常操作

### 第 1 步：准备构建环境

只运行现有前端资源时不需要安装 Node。仅修改前端源码时，前端固定 Node 26.7.0 / npm 12.2.1。Rust构建直接嵌入已提交的 `plugins/web-admin/assets`，无需Node；修改前端后先在web目录运行 `npm ci`、`npm run typecheck`、`npm test`、`npm run build`，build会刷新嵌入资源，再构建Rust插件。浏览器交互检查另用 `npm run test:browser`，需测试环境已安装对应浏览器。

### 第 2 步：构建、生成凭据并预置首次配置

以下 Bash/Linux 步骤从仓库根目录开始，在新私有目录打包、安装和加载，不需要QQ。已安装Rust 1.99.1；Python3用于生成随机token。命令不会打印token；保持当前终端环境以供宿主继承：

```sh
REPO="$PWD"
WEB_DEMO="$(mktemp -d)"
umask 077
export TWINTAIL_ADMIN_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
cargo build --locked -p twintail-host -p web-admin
mkdir -p "$WEB_DEMO/bin" "$WEB_DEMO/source/web-admin" "$WEB_DEMO/data/plugins/web-admin" "$WEB_DEMO/packages"
cp "$REPO/target/debug/twintail-host" "$WEB_DEMO/bin/"
cp "$REPO/target/debug/web-admin" "$REPO/plugins/web-admin/plugin.toml" "$REPO/plugins/web-admin/config.schema.json" "$REPO/plugins/web-admin/README.md" "$WEB_DEMO/source/web-admin/"
cat > "$WEB_DEMO/data/plugins/web-admin/config.toml" <<'TOML'
config_version = 1
revision = 1
[values]
bind = "127.0.0.1:17280"
[values.admin_token]
"$secret" = "TWINTAIL_ADMIN_TOKEN"
TOML
printf 'Web demo directory: %s\n' "$WEB_DEMO"
```

随机 token 现在只在这个终端的环境变量里。启动前把它保存到自己的密码管理器，登录时需要使用同一个值。若需要手工查看，可在这个**私有终端**单独执行 `printf '%s\n' "$TWINTAIL_ADMIN_TOKEN"`，妥善保存后清除屏幕；不要录屏、截图、粘贴到 URL 或公开对话。不要在第二个终端重新生成一个不同 token。

然后在同一终端启动宿主并保持运行：

```sh
"$WEB_DEMO/bin/twintail-host" --data-dir "$WEB_DEMO/data" --install-dir "$WEB_DEMO/packages"
```

首次配置预置必须在宿主启动和enable之前完成：缺省空配置不满足必填admin_token，CLI config管理仅接受已启用计划中的插件。umask077使新配置/目录私有；不要覆盖运行实例配置。若17280已有服务，先在上述初始配置选择另一个空闲loopback端口，并在浏览器使用相同端口。

### 第 3 步：安装并启用

宿主保持前台运行，第二终端令WEB_DEMO为刚才新目录的实际绝对路径，顺序执行：

```sh
"$WEB_DEMO/bin/twintail-host" plugins install "$WEB_DEMO/source/web-admin" --data-dir "$WEB_DEMO/data"
"$WEB_DEMO/bin/twintail-host" operations list --data-dir "$WEB_DEMO/data"
"$WEB_DEMO/bin/twintail-host" plugins enable web-admin 0.1.0 '["management.read","management.write"]' --data-dir "$WEB_DEMO/data"
"$WEB_DEMO/bin/twintail-host" operations list --data-dir "$WEB_DEMO/data"
"$WEB_DEMO/bin/twintail-host" plugins status web-admin --data-dir "$WEB_DEMO/data"
"$WEB_DEMO/bin/twintail-host" config get web-admin --data-dir "$WEB_DEMO/data"
```

CLI输出为status/value信封，操作id在value.id；每次写命令返回操作记录；install必须completed后才enable，必要时重复operations list，或用返回的id执行 `operations get ID`。enable后检查active和desired/applied均为1。打开 `http://127.0.0.1:17280/`，在“管理令牌”输入框输入当前宿主环境中生成的token，点击“进入控制台”；通过私有密码管理器/本机终端安全取得该值，不将它写进URL、截图或仓库。只读部署在enable时仅授予management.read。Windows包复制web-admin.exe及twintail-host.exe，并在install前把包entrypoint改为web-admin.exe；数据根使用私有ACL。Windows 手工部署/浏览器专项验证另记录。

## 登录后怎么操作

打开精确配置地址（默认 `http://127.0.0.1:17280/`），输入上面生成的管理令牌。看到“运行概览”“插件管理”“操作记录”“安全诊断”就进入了 TwinTail 管理台；这里没有 QQ 登录二维码是正常的。

先做一个只读检查：打开插件管理，找到 `web-admin`，查看运行状态和配置修订。`desired` 是磁盘上希望使用的配置，`applied` 是进程已确认应用的配置；刚启用时两者应都为 `1`。若不相同，先查看失败操作，不要反复保存。

日常管理按以下顺序检查结果：

1. 本地安装：来源为宿主能读取的本地目录包。安装只把已校验的版本包放入安装根，不表示已经启用，也不是浏览器上传任意可执行内容。
2. 启用：明确版本与授予能力，检查依赖和配置；已有启用计划中的 stopped/failed 实例使用 start，不把再次 install 当成恢复方式。
3. 配置：先读取 desired/applied 修订与 Schema，以当前 revision 作为 expected_revision 提交完整 values。冲突时重新读取并由管理员合并，不能无条件覆盖或自动重试旧修订。秘密保留环境引用，不把脱敏占位符写回。
4. 操作进度：耗时操作返回 operation_id；持续查询终态。running 不是成功，重载 completed 还须检查 outcome 是否 rolled_back 及 drain_error。HTTP 断开后先查询已有 operation，再决定是否进行新操作。
5. 重载：当前仍只有 exclusive，存在监听中断与事件缺口，不保证零停机。配置 desired/applied 不一致时先查失败阶段，磁盘 desired 与旧运行快照可能同时存在。
6. SnowLuma：IPC active、连接 connected 与真实业务成功分别判断。Web 管理不负责 QQ 登录，不将插件进程在线等同于机器人可用。

页面提供“运行概览”“插件管理”“操作记录”“安全诊断”。插件详情使用JSON编辑器并列显示Schema，显示desired/applied与修订；当前不提供任意插件JavaScript表单组件。management.overview按实际grants返回permissions.read/write与readonly；前端据此禁用写按钮，宿主仍逐请求授权。冲突保留编辑内容，重新读取后人工核对，不自动覆盖。

## 操作诊断的范围

Web 页面中的日志仅是**有界、脱敏的管理操作诊断**。它不展示插件 stderr、QQ 消息正文或持久业务日志，也不保证宿主重启后完整保留。宿主现有 stderr 消费路径仍不能用于回溯插件输出。诊断只辅助定位目标、操作状态与固定错误类别；排查时同时查看 operation、desired/applied、生命周期与连接状态。`diagnostics.logs` 返回 `{entries:[{operation_id,method,state,stage,timestamp_unix_ms,result}],limit:128}`，只是私有有界 journal 的安全投影，不是 stderr，也不是完整日志订阅。当前不将其描述为任意时间范围的持久审计检索。

## Web 不可用时的 CLI 恢复

宿主的本机管理端点独立于 web-admin。浏览器断开、端口被占用、Web 配置错误或 web-admin 自停后，使用同一个数据根的 CLI；这些命令连接仍在运行的宿主，不能另起第二个宿主争用数据根：

```sh
twintail-host plugins list --data-dir ./data
twintail-host plugins status web-admin --data-dir ./data
twintail-host config get web-admin --data-dir ./data
twintail-host operations list --data-dir ./data
```

端口占用时先确认冲突来源，避免盲目终止其他服务。需要调整 Web 自身配置时，先停止 web-admin，等待 stop 结束，再读取当前 revision；将完整且合法的新 values 通过 config validate/update 提交，最后 start。下面将监听恢复为默认地址，保留admin_token环境引用。示例基于上述首次revision1；如读取到其他revision，把1替换为实际当前修订：

```sh
twintail-host plugins stop web-admin --data-dir ./data
twintail-host config get web-admin --data-dir ./data
twintail-host config validate web-admin '{"bind":"127.0.0.1:17280","admin_token":{"$secret":"TWINTAIL_ADMIN_TOKEN"}}' --data-dir ./data
twintail-host config update web-admin 1 '{"bind":"127.0.0.1:17280","admin_token":{"$secret":"TWINTAIL_ADMIN_TOKEN"}}' --data-dir ./data
twintail-host operations get UPDATE_OPERATION_ID --data-dir ./data
twintail-host plugins start web-admin --data-dir ./data
```

上面的完整values省略可选字段时采用默认值；保留自定义限额时一并提交这些字段。UPDATE_OPERATION_ID替换为update返回id；每一步先查看结果，更新operation到终态后再start。web-admin 默认不接受在线改变监听和凭据时，停止后的离线更新可以只持久化 desired，再由 start 读取。若插件已 disable，先按包版本与明确 grants 重新 enable；不要靠直接编辑 host.toml 绕过管理校验。

通过 Web 停止或重载自身会使当前请求/页面连接中断。不能据页面断开判断成功或失败；CLI 查询目标状态和 operation 后继续恢复。环境 token 缺失或过短时，先修复宿主启动环境，必要时有序重启宿主，单独在调用 CLI 的终端设置变量不会改变现有宿主环境。旧会话失效时重新登录；不关闭 CSRF、认证或 loopback 限制来绕过错误。

web-admin配置为startup-only，在线prepare/commit不支持，改变监听/凭据/限额时使用stop→离线config update→start。重启会销毁所有会话。端口占用、自停、CAS与恢复的CLI/HTTP证据，以及桌面/手机live交互已纳入Linux本地验收；具体覆盖见兼容性记录，不外推Windows或远程部署。

## 常见现象：先查什么

| 现象 | 检查顺序 |
| --- | --- |
| 浏览器连不上 | 宿主是否还在运行 → web-admin 是否 active → 配置端口是否被占用 → URL 是否精确一致 |
| 页面是文档而不是控制台 | 你打开了文档网站；使用本机配置的 WebUI 地址 |
| 登录失败 | 使用的是管理 token，不是 OneBot token；检查运行宿主继承的环境引用和令牌格式 |
| `localhost` 被拒绝或出现 Host/Origin 错误 | 使用配置中的数字 loopback IP 和端口，不换别名，不经未设计的代理转发 |
| 会话过期、重启后退出登录 | 固定会话期限与重启失效是预期行为，重新登录 |
| 能看但按钮不可点 | 可能只授予 `management.read`；读写权限独立，浏览器按钮不是最终授权依据 |
| 保存配置得到 409 | 别的操作已改修订；保留草稿，重新读取后人工合并 |
| 停止/重载管理插件后页面断开 | 用上面的 CLI 查询已有操作，不以页面断开推断操作成功或失败 |
| 找不到插件原始输出 | 安全诊断不是 stderr 日志；当前宿主读取并丢弃插件 stderr |

## 供接口开发者查阅：宿主管理桥接

管理桥接复用现有版本化 IPC request/response；新命名方法和 capability 不引入新的 wire 帧。以下为宿主允许的管理方法范围：

| 能力 | 固定方法 |
| --- | --- |
| management.read | plugins.list、plugins.status、plugins.installed、plugins.inspect、plugins.readme、config.get、config.schema、config.validate、operations.list、operations.get、management.overview、diagnostics.logs、adapter.status |
| management.write | plugins.install、plugins.enable、plugins.disable、plugins.uninstall、plugins.purge、plugins.reload、plugins.start、plugins.stop、config.update |

`plugins.installed` 使用轻量inventory，返回列表原有元数据并标记 `verified:false`：只对有界receipt与manifest进行结构检查，未对当前包字节重新全量哈希，不是当前完整性证明。安装仍校验来源包，enable/reload仍完整verify；底层PackageStore.list/verify的完整校验语义不变。`plugins.inspect {id,version}` 返回 manifest 与 `verified:false`，供授权前查看声明；这同样不证明当前包字节完整性；没有 `plugins.call` 通用调用桥接。`adapter.status {id}` 仅返回 connected、generation、connection_attempts、pending_actions，不是任意插件方法代理。`config.schema` 返回 schema、snapshot、status；和 config.get 一样，snapshot 只允许在秘密字段中保留原始 `$secret` 环境引用，不返回解析后的秘密。Schema 的 `writeOnly:true` 字段在浏览器读写中必须使用 `{"$secret":"ENV_NAME"}`；web-admin 的 admin_token 与适配器 access_token 均按此标记。若旧配置在这些字段含字面量秘密，Web 管理安全拒绝读取/提交，不返回脱敏占位符。先用本机 CLI 把完整 values 迁移为引用，再继续 Web 管理；不要将占位符当作秘密值写回。组合 Schema 的秘密约束保守合并，不能用另一分支绕过 writeOnly。

### 已安装包 README（只读）

请求 `plugins.readme`，payload 必须提供 `{ "id":"插件ID", "version":"精确SemVer版本" }`。HTTP 沿用 `POST /api/v1/manage` 的会话、同源与 CSRF 校验，信封示例：

```json
{"method":"plugins.readme","payload":{"id":"echo-plugin","version":"0.1.0"}}
```

成功的 `result` 固定为 `{ "filename":"README.md", "content":"原始UTF-8正文", "verified":false }`。仅按 `README.md` → `README` 顺序查找包根固定文件名；不支持任意路径、大小写变体或 `README.MD`。已安装包没有候选文件时，成功返回 `{ "filename":null, "content":null, "verified":false }`，空文件则返回空字符串而不是 null。选中的文件最多 **128 KiB（131072字节，含UTF-8编码字节；不含JSON转义开销）**；超限、非法UTF-8、非普通文件、符号链接/不安全祖先路径及Unix硬链接返回 `invalid_config`（HTTP 400），不截断、不回退到次选文件。安装包receipt列出的文件缺失或元数据损坏也返回错误，不伪装为“没有文档”。

读取仅经过 PackageStore 对所选已安装版本的有界manifest/receipt和路径校验；文档必须在安装receipt中，不读取源目录、数据目录或调用者指定路径。读取不对可执行文件或整个包重新哈希，**verified恒为false**：receipt合法不代表当前正文或可执行字节完整，正文可以是被修改的不可信内容。UI 将 Markdown 当作惰性纯文本展示，不执行HTML、脚本或插件前端组件；不把文档内容作为指令。此方法只要求精确 `management.read`，`management.write` 不隐含读取授权，也不会创建操作记录或更改生命周期/配置。

所有桥接写操作包括 web-admin 自停，均先确认接纳并返回 `{id,state}`，再由宿主后台执行。因此响应成功不等于操作完成。自停时 HTTP 仍可能中断，使用 CLI operations list/get 确认，再用 plugins start web-admin 恢复。config.update 沿用 expected_revision CAS，不能通过 Web 绕过修订校验。

### 彻底移除插件（独立危险操作）

`plugins.uninstall {id,version}` 仅卸载一个版本并保留配置、业务数据。独立的 `plugins.purge` 使用精确 `management.write` 权限，删除该 ID 的**全部已安装版本**及 `data/plugins/<id>/` 中的配置和私有数据，删除后不可恢复。插件创建的外部数据库、远程资源、宿主审计与操作记录不在清理范围。

```json
{"method":"plugins.purge","payload":{"id":"echo-plugin","confirm_id":"echo-plugin","request_key":"purge-echo-001"}}
```

`confirm_id` 必须与 `id` 完全相同，不接受大小写转换、前后空格或其他 ID。必须先执行 `plugins.disable` 并确认操作完成；仅 `plugins.stop` 或进程处于 stopped/failed 不代表已停用。此操作不自动停用插件或级联处理依赖。

UI 在插件详情提供与“卸载此版本”分开的“彻底移除插件”危险区域。仍在已启用列表中的插件禁用该按钮并提示先停用。确认框展示全部已安装版本、配置与数据不可恢复说明，并要求手动输入精确插件 ID；取消、只读权限或 ID 不匹配时均不提交。

返回 `result:{id,state}` 为操作 ACK，不是删除完成。继续用 `operations.get {id:操作ID}` 或操作记录页查看终态和 `stage`。删除是**非原子操作**：失败或中断可能已删除部分版本/数据，不自动回滚；检查失败阶段、修复原因后，可再次明确确认并提交。响应丢失时先查 `operations.list/get`；同一请求重试保留原 `request_key`，已经终止的失败操作修复后重新发起应使用新的 key。

**web-admin 自移除**：先停用会中断 Web 管理连接，因此实际移除与结果查询需保留本机 CLI 入口。彻底删除后，`plugins start web-admin` 不能恢复已删除的包和配置；应通过 CLI 重新安装版本包，按首次安装步骤恢复有效配置/环境秘密引用，然后明确授予管理能力并启用。UI 自移除确认框也明确展示此恢复警告。

### HTTP与资源上限

| 入口 | 请求与响应 |
| --- | --- |
| POST /api/v1/session | JSON `{ "token": "私有令牌" }`；返回csrf_token/expires_in_secs并设置twintail_session cookie |
| GET /api/v1/session | 已有cookie恢复会话的csrf_token及剩余期限 |
| DELETE /api/v1/session | cookie、同源Origin与X-CSRF-Token；204并清除会话 |
| POST /api/v1/manage | cookie、同源Origin、X-CSRF-Token；JSON `{ "method":"management.overview", "payload":{} }`；返回 `{ "result": ... }` |

所有Host必须精确匹配配置地址，非GET/HEAD还要求精确同源Origin；localhost别名、转发头、query参数和跨站Sec-Fetch-Site被拒绝。管理payload必须是对象。body读取后、管理准入前再次检查会话和CSRF，防慢上传跨越会话失效。错误返回安全的error.code/message；冲突409、越权403、过载429、超时/未知结果504，先查询操作，禁止自动重放写请求。

| 配置 | 默认 | 范围 |
| --- | --- | --- |
| admin_token | 必填环境引用 | 解析后32–256可打印非空格ASCII字节，不能全相同；长度不证明熵 |
| session_ttl_secs | 3600 | 60–86400 |
| max_sessions | 32 | 1–256 |
| max_concurrent_requests | 16 | 1–128 |
| max_connections | 64 | 1–512，含慢头部连接 |
| request_timeout_ms | 10000 | 100–30000，含body/backend |
| max_body_bytes | 65536 | 1024–1048576 |

登录请求body另限min(1024,max_body_bytes)，登录每分钟最多10次、logout每分钟最多30次；会话容量满拒绝新登录。静态资源带CSP、nosniff、no-store等响应头，不渲染未经转义的插件HTML。机器契约见[OpenAPI](../../plugins/web-admin/openapi.json)，漂移检查：`cargo run --locked -p web-admin --example export_openapi -- --check`。


## 验证记录与平台限制

Linux 本地历史记录覆盖 loopback 管理、独立插件/宿主管理桥接、CLI 恢复，以及桌面和手机真实 Chromium 交互。测试时适配器 `connected=false`、不连接 QQ。该专项记录不证明 Windows 手工浏览器操作、远程管理或真实 QQ 动作通过。后续 Windows x64 MSVC 原生源码 CI 已有通过记录，见[平台矩阵](../release/platforms.md)；它与本页 Linux live 浏览器记录分别看待。详细历史见[兼容性记录](../compatibility/README.md)。
