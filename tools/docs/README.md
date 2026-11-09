# 文档网站工具

文档单一来源是仓库 `docs/` 中的 Markdown；除首页和样式外，不制作网站副本。导航在 `mkdocs.yml`，固定构建入口依赖在 `requirements-docs.txt`。

```sh
python3 -m venv /tmp/twintail-docs-venv
/tmp/twintail-docs-venv/bin/python -m pip install -r requirements-docs.txt
/tmp/twintail-docs-venv/bin/python tools/docs/check.py
```

`check.py` 先执行 `mkdocs build --strict`，再检查所有生成 HTML 的本地链接、资源和锚点；默认使用临时目录并自动清理。外部 URL 不发起请求，不需要凭据。可检查已有构建：

```sh
python tools/docs/check.py --site-dir site
```

本地预览与正式产物：

```sh
python -m mkdocs serve
python -m mkdocs build --strict
```

默认产物为 `site/`，部署基路径是 `https://Eetherrr.github.io/TwinTail/`。内建搜索使用本地生成的索引，字体使用系统字体，不请求 Google Fonts。Material 自带 Mermaid 前端支持；图表按主题默认机制加载，普通正文阅读与搜索不依赖它。

`hooks.py` 在 Markdown 解析链接后、MkDocs 路径处理前，将越出 `docs/` 的仓库内文件链接转为 GitHub `blob/main` URL，目录使用 `tree/main`；保留查询参数和锚点，先校验目标存在且仍属于仓库。`docs/` 内的相对链接、README 路由与锚点由 MkDocs 正常处理，因此源文档继续同时适用于 GitHub 和网站。引用式链接与图片链接也经过相同处理，代码示例不受文本替换影响。外部 URL 与绝对 URL 不重写。

新增页面时同步导航。不要通过降低 `validation` 级别或删除锚点来隐藏链接错误。编辑链接始终指向 `edit/main/docs/` 对应源文件。
