"""Keep repository Markdown unchanged while resolving links outside docs/.

Run before MkDocs' relative-path processor: internal links (including README.md
and fragments) remain MkDocs' responsibility. References, images and ordinary
Markdown links all reach this treeprocessor after Markdown's inline parsing.
"""
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit, urlunsplit

from markdown.extensions import Extension
from markdown.treeprocessors import Treeprocessor
from mkdocs.exceptions import PluginError


class RepositoryLinks(Treeprocessor):
    def __init__(self, md, context):
        super().__init__(md)
        self.context = context

    def run(self, root):
        docs, repo, source = self.context
        for element in root.iter():
            attribute = {"a": "href", "img": "src"}.get(element.tag)
            if not attribute or attribute not in element.attrib:
                continue
            value = element.get(attribute)
            parts = urlsplit(value)
            if parts.scheme or parts.netloc or not parts.path or parts.path.startswith("/"):
                continue
            target = (source.parent / unquote(parts.path)).resolve()
            if target.is_relative_to(docs):
                continue
            if not target.is_relative_to(repo) or not target.exists():
                raise PluginError(f"{source.relative_to(repo)}: missing repository link: {value}")
            kind = "tree" if target.is_dir() else "blob"
            path = quote(target.relative_to(repo).as_posix(), safe="/")
            element.set(attribute, urlunsplit(("https", "github.com", f"/Eetherrr/TwinTail/{kind}/main/{path}", parts.query, parts.fragment)))


class RepositoryLinkExtension(Extension):
    context = None

    def extendMarkdown(self, md):
        # MkDocs registers its relative-path processor at priority 0.
        md.treeprocessors.register(RepositoryLinks(md, self.context), "twintail_repository_links", 5)


_extension = RepositoryLinkExtension()


def on_config(config):
    config.markdown_extensions.append(_extension)
    return config


def on_page_markdown(markdown, page, config, files):
    docs = Path(config.docs_dir).resolve()
    _extension.context = (docs, Path(config.config_file_path).resolve().parent, Path(page.file.abs_src_path).resolve())
    return markdown
