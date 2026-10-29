# ADR 0005：P4 OneBot v11 与 SnowLuma 正向 WebSocket

- 状态：设计已定，实施中，待实际代码与验收校正
- 日期：2026-10-29
- 依据：AGENT.md、SKILL.md、ADR 0003/0004、OneBot v11 公开规范

## 决策

建立独立 `crates/twintail-onebot11` 协议类型库与 `adapters/adapter-snowluma` 插件。协议库不依赖宿主或具体 QQ 登录实现；宿主不链接 SnowLuma 代码。SnowLuma 独立部署，插件只连接它提供的 OneBot 服务。

P4 仅实现正向 Universal WebSocket：适配器作为客户端，在同一 ws/wss 连接接收事件、发送动作并接收响应。反向连接、split API/Event 与 HTTP 延后，不使用“完整支持 v11”描述 MVP。路径和端口由配置指定。鉴权使用 Authorization Bearer，不把 token 放 URL。wss 验证服务端证书，不以测试环境证书问题引入跳过校验开关。

配置 self_id 使用字符串，并在接入边界接受合法数值标识的无精度损失规范化。先绑定配置账号，再拒绝不匹配的事件身份；不能从任意远端事件自动注册其他账号。机器人路由使用宿主绑定的适配器 instance 与 self_id。配置名称、限额与重连参数在实现落盘后校正。

解析覆盖 message、notice、request、meta_event，保留原始 JSON、未知扩展字段和消息段。CQ 字符串原样保留，不静默转换为文本段或拼接 CQ。按结构识别响应和事件，不能只凭 echo 是否存在分类；未知事件与响应扩展不丢弃，但不将未支持语义声明为已处理能力。

每次 WS 连接拥有独立 generation，echo 仅在该连接内关联有界 pending。迟到响应、未知 echo 和旧连接响应不能完成新连接调用。事件队列、发送队列、pending、消息大小与处理并发均有界；慢消费者导致显式缺口，不设置持久重放队列。WS ping/pong 与 OneBot heartbeat 是不同机制。

动作发送前的离线、校验或准入失败返回明确错误；准入后超时、断开或取消可能已在远端执行，保守返回 UnknownOutcome。保留 OneBot status、retcode、data 与扩展响应供调用者判断远端失败，不把 retcode 失败改成传输成功。断线重连可恢复未来请求，但不重放已准入动作，不承诺事件补发或 exactly-once。

插件注册 `onebot.v11` 版本化服务，提供身份与连接信息；SDK 自定义 request `connection.status` 区分 IPC active 和外部连接可用。具体方法及返回字段以实现校正，秘密与完整含 token URL 不进入诊断。配置、任务和连接生命周期沿用 P3：exclusive 替换，停止时关闭连接、完成 pending 清理并 join 后台任务，旧 BotRef 失效。在线配置可拒绝需要重连的变更，不以 prepare 执行外部副作用。

## 验收与兼容性边界

无凭据夹具分别验证四类事件、未知字段/CQ 保留、身份不匹配、鉴权、响应分类、echo/代次、背压、断线、超时、retcode 和优雅停止。mock WS 与独立进程闭环仅证明 TwinTail 实现，不能证明真实 SnowLuma 兼容。

真实验收记录 SnowLuma 返回版本、TwinTail 测试源码、客户端平台、传输和日期，对未知运行提交/发行包摘要/远端 OS 明确标未知。2026-10-29 已完成 Linux x64 WSL mirrored 客户端对 SnowLuma 2.2.1-node 指定部署的本地与真实范围验收：直接查询/登录/私聊，以及 Supervisor→独立 adapter→临时 feature 在断线重连后的收消息→业务→单次回复，用户确认收件。远端 OS/commit/SHA 未知，notice/request、群聊、其他动作、真实 Bearer/TLS和其他平台仍待测，不以局部能力外推完整兼容。实际证据与矩阵见[兼容记录](../compatibility/snowluma.md)，作者契约见[指南](../plugin-api/adapters.md)。

## 外部依据

- [OneBot v11](https://github.com/botuniverse/onebot-11)
- [正向 WebSocket](https://github.com/botuniverse/onebot-11/blob/master/communication/ws.md)
- [鉴权](https://github.com/botuniverse/onebot-11/blob/master/communication/authorization.md)
- [事件](https://github.com/botuniverse/onebot-11/blob/master/event/README.md)
- [动作 API](https://github.com/botuniverse/onebot-11/blob/master/api/README.md)
- [SnowLuma](https://github.com/SnowLuma/SnowLuma)

公开版本信息和规范正文核对结果另记兼容页；产品声明不是本机运行证据。
