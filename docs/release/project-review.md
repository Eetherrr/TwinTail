# 项目审查与文档站点

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

## 文档维护与部署

网站直接使用 `docs/` Markdown；Python 依赖由根目录 `requirements-docs.txt` 固定 MkDocs 与 Material 版本。构建操作在仓库根目录执行：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r requirements-docs.txt
python tools/docs/check.py
python -m mkdocs serve
```

`check.py` 在临时目录执行严格构建，再检查生成 HTML 的本地链接、资源与锚点；不联网验证外链。仓库根文档、源码等站外链接在构建时映射到 GitHub main，对应已发布制品时应另行核对版本。

GitHub Actions 的 Documentation / GitHub Pages 工作流对 PR 执行构建，对 main 的文档变更构建并部署。首次启用需在仓库 Settings → Pages 中将 Source 设为 **GitHub Actions**；工作流会尝试初始化，若仓库策略限制自动启用，需由管理员完成该设置后重新运行。预期项目站点地址是 <https://eetherrr.github.io/TwinTail/>，实际上线以成功部署和公开访问为准。

网站无后端、账号或 Bot 管理能力；它与本机 WebUI 是两个独立入口。不要在文档里提交登录 token、个人账号或运行数据。

## 相关入口

- [平台矩阵](platforms.md)
- [发行指南](README.md)
- [安全边界](../architecture/security.md)
- [变更记录](../../CHANGELOG.md)
- [GitHub Pages 工作流](../../.github/workflows/docs.yml)
