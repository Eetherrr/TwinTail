# echo-plugin

## 用途与能力

最小 Rust SDK 业务插件示例：启动时精确订阅 `message`，从事件复制 `message`、`message_type`、`user_id`、`group_id` 字段，通过该事件的 BotRef 调用 `send_msg`。没有 `message` 字段时忽略事件；消息段按 JSON 原样传递，不解析 CQ 码、不做命令过滤或去重。`echo.ready` 请求用于检查订阅是否完成。

清单版本 `0.1.0`，必须由宿主授予 `events.subscribe`、`actions.call`。随包清单强依赖 `mock-adapter ^0.1.0`，默认用于合成事件演示，不直接连接 QQ。若用于真实适配器，应在生成/安装包前准备匹配依赖的独立清单与摘要，并限制测试消息，避免机器人之间互相回声。

## 配置与使用

没有插件自定义配置 Schema 或配置字段，不声明私有存储能力。由宿主提供生命周期和配置快照；自身不持久化状态。先构建两个示例与宿主，或使用对应平台发行包。每个包至少含清单和可执行文件；发行包另附 README 与摘要。

宿主运行后，先安装、启用 mock-adapter，再安装、启用本插件：

```sh
twintail-host plugins install MOCK_PACKAGE_DIR --data-dir DATA_DIR
twintail-host plugins enable mock-adapter 0.1.0 '["bots.register","events.publish","services.provide"]' --data-dir DATA_DIR
twintail-host plugins install ECHO_PACKAGE_DIR --data-dir DATA_DIR
twintail-host plugins enable echo-plugin 0.1.0 '["events.subscribe","actions.call"]' --data-dir DATA_DIR
twintail-host plugins call mock-adapter mock.emit '{"message_type":"private","user_id":"10002","message":[{"type":"text","data":{"text":"hello"}}]}' --data-dir DATA_DIR
twintail-host plugins call mock-adapter mock.actions '{}' --data-dir DATA_DIR
```

每个 install/enable 返回 operation_id，必须先用 `operations get OPERATION_ID` 确认完成再进行下一步。最后应看到 `send_msg` 动作记录；事件发布返回的 delivered 只是入队计数，不是业务处理完成确认。

## 生命周期与重载

通过宿主启停；manifest 未显式设置 reload_policy，使用默认 `exclusive`，替换时重新订阅，不承诺零事件丢失。回声发送超时可能已执行，不应自动重试。ready 状态只在内存中，没有数据迁移或回滚逻辑。活动强依赖关系会限制 mock-adapter 的替换，先停止消费者再处理提供者。
