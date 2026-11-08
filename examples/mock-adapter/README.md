# mock-adapter

## 用途与能力

本地合成适配器示例，不连接 QQ 或 SnowLuma。启动时注册账号 `10001` 的 BotRef 与 `mock.inspect@1.0.0` 服务；服务返回调用参数。常与 echo-plugin 配合测试宿主事件/动作路由。

清单版本 `0.1.0`，必须由宿主授予 `bots.register`、`events.publish`、`services.provide`。调试请求：`mock.ready` 返回就绪状态和 BotRef；`mock.emit` 将参数作为 `mock.v1` 的 `message` 事件载荷发布；`mock.actions` 查询收到的动作；`mock.health` 查询宿主健康。`mock.action`/`mock.service` 是 SDK 调用透传测试入口，仍受宿主能力校验；默认清单没有申请 `actions.call`/`services.call`，不会因存在这些入口自动获得权限。

收到的动作仅记录在内存中，返回模拟 OneBot 成功结果与递增 message_id，不执行真实发送；最多记录 1024 条，满后返回 Overloaded。

## 配置与使用

没有自定义配置 Schema 或配置字段，无私有存储能力或持久化数据。构建 `mock-adapter` 可执行文件并与清单组成包，或使用对应平台发行包（含 README 与摘要）。宿主运行后：

```sh
twintail-host plugins install PACKAGE_DIR --data-dir DATA_DIR
twintail-host plugins enable mock-adapter 0.1.0 '["bots.register","events.publish","services.provide"]' --data-dir DATA_DIR
twintail-host plugins call mock-adapter mock.ready '{}' --data-dir DATA_DIR
twintail-host plugins call mock-adapter mock.emit '{"message_type":"private","user_id":"10002","message":[{"type":"text","data":{"text":"hello"}}]}' --data-dir DATA_DIR
twintail-host plugins call mock-adapter mock.actions '{}' --data-dir DATA_DIR
```

每个 install/enable 返回 operation_id，用 `operations get OPERATION_ID` 等待终态后进行下一步。要观察动作闭环，须另启用已授权的 echo-plugin 等消息消费者；单独发布事件不产生回声。发布返回 delivered/dropped 入队统计，不保证消费者已经处理。

## 生命周期与重载

manifest 未显式设置 reload_policy，采用默认 `exclusive`。启停或重载会重新注册 BotRef/服务并清空动作历史，新代次不沿用旧 BotRef。没有数据迁移、跨重载恢复或零丢事件承诺。有活动强依赖消费者（例如默认 echo-plugin）时，宿主阻止替换提供者，应先停止消费者再重载。插件只通过宿主生命周期运行，stdout 专用于 IPC，诊断写入 stderr。
