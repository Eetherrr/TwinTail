# 在本地阅读、预览和部署文档网站

这里说的是**你正在阅读的静态说明网站**，不是管理机器人的 WebUI。它没有后端、登录账号或 Bot 管理功能；不会启动宿主、安装插件、连接 QQ，也不需要管理 token。

## 你可以做什么

- 直接在仓库阅读 Markdown，不安装任何网站工具。
- 在自己电脑运行预览，边改文档边查看排版和链接。
- 将文档发布到 GitHub Pages，让其他人通过浏览器阅读。

**MkDocs** 是把 Markdown 转为网页的工具，**Material** 是网站主题，**GitHub Pages** 是托管这些静态网页的服务。它们都不是 SnowLuma，也不是 TwinTail 的 `web-admin` 插件。

## 第 1 步：准备本地环境

准备完整源码、Python 3 与 `venv/pip`，在仓库根打开终端。依赖下载需要网络；根目录 `requirements-docs.txt` 固定网站构建依赖。以下使用 Bash/Linux 风格激活命令，Windows 虚拟环境激活路径不同。

## 第 2 步：安装依赖、检查并预览

网站直接使用 `docs/` Markdown；Python 依赖由根目录 `requirements-docs.txt` 固定 MkDocs 与 Material 版本。构建操作在仓库根目录执行：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r requirements-docs.txt
python tools/docs/check.py
python -m mkdocs serve
```

运行 `mkdocs serve` 后保持终端打开，在浏览器访问其实际打印的本机地址（通常为 `http://127.0.0.1:8000/`）；看到与本文档相同导航即为预览成功。Ctrl-C 停止预览。此地址不是 WebUI 的 `17280` 管理端口。

`check.py` 在临时目录执行严格构建，再检查生成 HTML 的本地链接、资源与锚点；不联网验证外链。仓库根文档、源码等站外链接在构建时映射到 GitHub main，对应已发布制品时应另行核对版本。

## 第 3 步：需要公开网站时部署 Pages

GitHub Actions 的 Documentation / GitHub Pages 工作流对 PR 执行构建，对 main 的文档变更构建并部署。首次启用需在仓库 Settings → Pages 中将 Source 设为 **GitHub Actions**；默认工作流令牌不具备首次启用 Pages 的仓库管理权限，须由管理员完成该设置后运行工作流，不需要向构建环境提供个人令牌。预期项目站点地址是 <https://eetherrr.github.io/TwinTail/>，实际上线以成功部署和公开访问为准。

网站无后端、账号或 Bot 管理能力；它与本机 WebUI 是两个独立入口。不要在文档里提交登录 token、个人账号或运行数据。



## 成功标志与故障排查

| 现象 | 怎么查 |
| --- | --- |
| 本地检查返回非零 | 看具体页面、链接或锚点错误；先修文档再构建，不靠忽略严格检查发布 |
| 找不到 mkdocs 或主题 | 激活同一个虚拟环境，确认 requirements 安装成功 |
| 预览地址打不开 | 预览进程应仍在运行；以终端实际打印地址为准，检查本机端口占用 |
| Pages 工作流失败 | 打开 Actions 失败步骤，先核对 Settings → Pages 的 Source 与工作流权限 |
| 公开地址 404 | 确认部署成功与实际站点地址，不能只凭本地 build 成功宣布上线 |
| 页面没有机器人登录和插件按钮 | 正常，这是说明网站；运行管理插件后另开 TwinTail WebUI |

不要在文档提交 token、个人账号或运行数据。链接到 GitHub main 的源码可能比已下载的旧发行包更新，排查具体版本时要切换到相应 tag。

## 维护审查记录（2026-11-09）

下文是文档事实核对记录，不是第一次阅读必须理解的操作步骤。

## 2026-11-09 审查范围

本次以源码 `8c3d096a88cdf40d77d1c3ce99b3ce3bd934d800` 为基线，审查架构入口、SDK/管理接口说明、平台验证记录、发行说明及文档发布方式。采用源码与现有 CI 证据交叉核对；不是全量安全审计，也未重新连接真实 QQ。

## 已修正文档问题

| 问题 | 修正 |
| --- | --- |
| MIT 与公开发行仍被描述为准备中 | 明确 v0.1.0 已发行，main 的 Unreleased 功能不自动进入已有附件 |
| Windows 仍被笼统标为待验证 | 指向最新原生 CI 矩阵，保留跨用户 ACL、真实 QQ、live 浏览器等独立限制 |
| 反向 WS 被写为未来功能 | 明确 loopback reverse Universal WS 已实现，真实 SnowLuma 反向部署尚未验证 |
| quickstart 否认配置事务等后续能力 | 将静态包教程标为特定演示路径，链接当前配置、包管理及重载文档 |
| 库存被误写为全量摘要验证 | 区分 metadata-only `verified: false` 与加载前完整校验 |

彻底移除的确认参数、先停用要求、阶段与部分失败重试说明，已与宿主管理实现核对；本次没有确认新的 purge API 缺陷。此结论不代表覆盖所有竞态或所有平台文件系统行为。

## 保留的边界

- 插件是受信任的普通子进程，不是同用户文件系统沙箱；宿主管理目录外资源不保证自动清理。
- 真正 SnowLuma 兼容范围保持既有专项证据，不由 mock 或 CI 扩大。
- 包摘要用于内容变化检测，不是来源签名。
- 最新源码 CI 与 v0.1.0 附件的版本、测试数量分别记录，不混用。
- 依赖审查仍应结合[历史报告](dependency-audit.md)和更新后的数据库；本次没有运行新的依赖漏洞扫描。


## 相关入口

- [平台矩阵](platforms.md)
- [发行指南](README.md)
- [安全边界](../architecture/security.md)
- [变更记录](../../CHANGELOG.md)
- [GitHub Pages 工作流](../../.github/workflows/docs.yml)
