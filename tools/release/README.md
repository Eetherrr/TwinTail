# 本地候选打包工具

Python 3.11+，仅使用标准库。当前 workspace 0.1.0 为工程版本，产物始终是 `channel: candidate`、`public_release: false`、`license_gate: MIT`、`license: MIT`。项目已由权利人授权采用 MIT，候选必须携带 `LICENSE`、`THIRD_PARTY_NOTICES.md` 和第三方许可正文；第三方依赖继续按其各自许可使用。工具不上传、不打 tag、不执行 cargo publish，远端发行由独立授权流程执行。参见[发行指南](../../docs/release/README.md)。

从仓库根目录运行；所有输出必须是受信任用户独占的私有目录中的新路径。不要让其他用户并发写入输入、输出及其父目录。预存输出、符号链接和 Windows reparse point/junction会拒绝；标准库检查和 rename 不能防止同用户恶意并发替换路径。构建不检查整个 Cargo 缓存目录树，stage 只读取指定的五个最终二进制。

```sh
python3 tools/release/release.py build --target linux-x86_64
# 已在 CI 构建时可跳过 build，直接使用该 runner 的预构建目录。
python3 tools/release/release.py stage --target linux-x86_64 --binaries target/x86_64-unknown-linux-gnu/release --output /private/newstage
python3 tools/release/release.py archive --stage /private/newstage --output /private/twintail-0.1.0-linux-x86_64.tar.gz
python3 tools/release/release.py verify /private/twintail-0.1.0-linux-x86_64.tar.gz
python3 tools/release/release.py verify /private/newstage
python3 -m unittest discover -s tools/release -p 'test_*.py' -v
```

归档包含维护脚本及文档，但不含完整 Rust 源码和全部开发示例源码。`build`、契约插件脚手架及文档中的 Cargo/example 命令须使用对应版本的完整 source checkout；在解包目录内直接执行 build 不成立。可用 `python3 tools/release/release.py --repo /private/source-checkout build --target ...` 显式指向源码。

`--repo PATH` 是放在子命令前的可选全局参数，默认当前脚本所属仓库；`build --target-dir PATH` 默认 `target`。`build` 调用 `cargo build --locked --release --target TRIPLE --target-dir PATH` 并明确选择 host、adapter-snowluma、web-admin、echo-plugin、mock-adapter；最终前端静态资源已由 web-admin 构建脚本嵌入，打包不运行 npm。须事先安装 Rust target 及对应原生链接器。各子命令成功时 stdout 输出一条 JSON，失败返回非零并输出固定安全错误，不打印输入秘密或解析异常详情（Cargo 构建诊断属于 Cargo 自身输出）。

| `--target` | Rust triple / `target/TRIPLE/release` | 二进制 / 归档 |
| --- | --- | --- |
| `linux-x86_64` | `x86_64-unknown-linux-gnu` | ELF64 x86_64 / `.tar.gz` |
| `linux-aarch64` | `aarch64-unknown-linux-gnu` | ELF64 aarch64 / `.tar.gz` |
| `windows-x86_64` | `x86_64-pc-windows-msvc` | PE x86_64 `.exe` / `.zip` |

合成测试能在任意 runner 构造三个目标的头部用于检查打包分支，不能证明对应系统二进制可以执行。Linux x86_64 的实际构建/归档证据与 Windows/ARM 原生运行证据分别记录。`release.json.runtime_verified` 固定为 false；verify 只证明结构与摘要一致，不认证 OS 运行状态或构建来源。跨平台 libc/系统最低版本仍需目标 runner 验证。Windows MSVC 发行目标与历史 GNU 编译检查不是同一项证据。

产物布局：

```text
twintail-0.1.0-TARGET/             # 仅归档有顶层目录
  bin/twintail-host[.exe]
  packages/PLUGIN/0.1.0/
    PLUGIN[.exe]
    plugin.toml
    checksums.json
    config.schema.json            # web-admin 和 adapter-snowluma
  docs/{architecture,plugin-api,compatibility,release}/...
  schemas/*-v1.json
  plugins/web-admin/openapi.json
  web/{package.json,package-lock.json}
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  README.md / AGENT.md / SKILL.md / CHANGELOG.md
  LICENSE
  THIRD_PARTY_NOTICES.md
  third-party-licenses/{rust-dependencies.txt,web-dependencies.txt}
  SECURITY.md
  tools/state/{state.py,README.md}
  tools/release/{release.py,README.md}
  release.json
  SHA256SUMS
```

stage 使用代码中的精确文件白名单，不递归复制源码仓库或 target；fixtures、运行配置、data、令牌、`.env` 和缓存不会从额外路径混入。五个输入二进制检查 ELF/PE 目标架构；清单 id/version 必须匹配 workspace，entrypoint 必须为原始插件名，打包后重写 Windows `.exe` 和三个已定义的 manifest `target` 值。发布包不会启用插件、不生成 host.toml、不授予能力，也不包含 SnowLuma 本体。四插件附带兼容核心 PackageStore 的 `checksums.json`，可作为独立本地包源安装。

`SHA256SUMS` 按路径排序，覆盖所有其他文件，包括 `release.json` 和插件 receipt，排除自身以避免递归；插件 receipt 覆盖所在包的 manifest、binary 和可选 schema，排除自身。元信息记录版本、Rust MSRV、target/triple、release profile、两个 lock 摘要和候选 gate。SHA256 是损坏检测，不是签名或供应链来源认证；攻击者同时重写文件和摘要仍可伪造。MIT 和第三方许可正文随包提供，摘要验证不代替遵守这些许可。工具不签名。

stage 文件权限统一为 binary 0755、其他 0644，文件时间为 epoch；归档按路径排序，tar owner/group/time 清零、无扩展头；gzip 不含文件名、mtime 为零；ZIP 时间为 1980-01-01、文件权限固定、没有额外字段。相同内容、相同 Python/zlib 实现生成相同归档摘要；不声称不同 Rust 构建环境的二进制或不同压缩库版本逐字节相同。输出目录名不影响归档。归档顶层名由版本/target 固定生成，建议输出文件名也带相同信息。

verify 直接有界读取目录或归档，不自动解包，不执行二进制；拒绝路径穿越、绝对路径、链接、special files、重复成员、不同顶层名、非标准元数据、缺失/多余文件、未知空目录和摘要不符。POSIX 目录额外检查执行权限；Windows 文件系统不提供相同 POSIX 权限语义，归档记录仍检查。上限 256 文件、单文件 128 MiB、总内容 512 MiB，归档输入 512 MiB；这不替代核心安装包的更严格上限。输入及输出必须在受保护目录中保持稳定。

先 verify 归档成功，再使用当前平台受支持的归档工具解包至新私有目录，随后对解包目录再次 verify。归档内安装包仍由 host 启用流程校验 manifest/target/schema/权限和 hash。安装与启用独立；运行参数、秘密注入、迁移备份、升级回滚见[运维说明](../../docs/release/operations.md)。
