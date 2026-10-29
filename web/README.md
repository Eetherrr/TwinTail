# TwinTail 管理前端

中文 Vue 3 / TypeScript / Vite 单页管理台。Node 与 npm 版本见 package.json；直接依赖均固定版本，提交 package-lock.json。

```sh
npm ci
npm test
npm run build
```

构建包含 vue-tsc 严格类型检查，输出 web/dist，同时将可嵌入的 index.html、assets/app.css 和 assets/app.js 复制到 plugins/web-admin/assets。dist、node_modules 和浏览器临时结果不提交；嵌入资源由项目约定跟踪。重新构建 Rust web-admin 后资源才更新。

```sh
npx playwright install chromium
npm run test:browser
```

浏览器测试包含桌面和手机尺寸。默认模拟 HTTP 合同，覆盖确认后停止、异步配置冲突保留、秘密引用保留、只读控制、诊断与横向溢出。真实宿主测试仅在显式设置 TWINTAIL_WEB_BASE_URL 和 TWINTAIL_WEB_TOKEN_FILE 时启用；必须使用隔离宿主，因为测试会对 echo-plugin 执行配置 CAS（先验证在线应用不支持时明确失败并保留草稿，再停止插件成功保存同值配置、重新读取递增修订）、启动、重载及 desired/applied 修订一致性。token 从受保护文件读取，不放在命令行、浏览器存储或截图中。浏览器追踪关闭，避免登录请求令牌进入归档。若已有 Chromium，可通过 PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH 指定二进制。

真实 mock-adapter 不具备正式适配器状态能力，live 测试断言其请求错误清晰可见且日志刷新后保留；模拟 HTTP 场景验证成功状态摘要。隔离宿主若提供 adapter-snowluma，live 测试也会验证真实 connection.status 的 connected=false 摘要；使用不可达本地地址，不建立外部服务连接。真实适配器连接需要相应的适配器测试环境。

HTTP 使用同源 Cookie 会话和 X-CSRF-Token。配置草稿保持原基准 revision，后台轮询只更新 desired/applied 状态；失败和冲突保留草稿，通过显式确认重新读取替换。浏览器不解析秘密引用，阻止常见脱敏占位符及无法精确表示的整数提交。写权限未知时禁用特权操作。页面只使用 Vue 文本插值，不注入后端 HTML。
