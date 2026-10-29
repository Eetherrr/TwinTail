# P6 依赖审计与公开发行门槛

审计日期：2026-10-29；执行环境为 Linux，Rust 1.99.1、Node v26.7.0、npm 12.2.1。首次 OSV 响应时间为 `2026-10-29T13:28:10.824110Z`（北京时间 21:28:10），复核时间为同日 13:31–13:40 UTC。本文记录一次实际锁文件审计，不代表未来依赖或平台持续安全认证。

## 输入与范围

- Cargo.lock SHA-256：`ab1c173f091073c63d6017c3c3c7c312ccece830af88db5ba3b6039d5e7f30a2`。
- web/package-lock.json SHA-256：`b89c172e28c4e5264b6959406d75760e18836de9122c3ebdc96d3572b86ddd95`。
- Rust 审计仅发送锁文件中 registry 依赖的公开包名、版本给官方 `https://api.osv.dev/v1/querybatch`；未发送本地包源码、配置、路径、账户或秘密。共查询 206 个 registry 包。
- 未安装 cargo-audit；本次使用官方 OSV API，并进一步核对原始 advisory 与锁定源码。OSV 不是 cargo-audit/RustSec 数据库全功能等价替代，不覆盖未知漏洞、许可证兼容、恶意发布或运行态配置错误。
- 没有更新任何依赖或锁文件；没有执行 `npm audit fix`，也没有为消除扫描结果盲目升级 breaking 版本。

## 实际命令与结果

### Cargo 锁文件

等价可复核命令（Python 3.11+，仅公开依赖数据）：

```sh
python3 - <<'PY'
import json, tomllib, urllib.request
with open('Cargo.lock', 'rb') as stream:
    lock = tomllib.load(stream)
packages = [p for p in lock['package'] if p.get('source', '').startswith('registry+')]
query = {'queries': [
    {'package': {'name': p['name'], 'ecosystem': 'crates.io'}, 'version': p['version']}
    for p in packages
]}
request = urllib.request.Request(
    'https://api.osv.dev/v1/querybatch',
    data=json.dumps(query).encode(), headers={'Content-Type': 'application/json'})
with urllib.request.urlopen(request, timeout=60) as response:
    result = json.load(response)
for package, entry in zip(packages, result['results']):
    if entry.get('vulns'):
        print(package['name'], package['version'], entry['vulns'])
print('queried', len(packages))
PY
cargo tree --locked --offline -i block-buffer
```

实际 API 请求成功，查询 206 个包；仅命中 `block-buffer 0.10.4`：**GHSA-qwgh-2vcv-g2f7**，上游等级 MODERATE。实际 `cargo tree -i block-buffer` 显示反向依赖为 `digest 0.10.7` → `sha2 0.10.9` → twintail-core/twintail-host（包完整性及管理相关摘要）；未将其他主版本的 SHA 实现误列入这条依赖链。不将命中直接当作本项目已有可利用攻击路径。

### advisory 适用性复核

- 官方上游：[RustCrypto GHSA-qwgh-2vcv-g2f7](https://github.com/RustCrypto/utils/security/advisories/GHSA-qwgh-2vcv-g2f7)。通过网页抓取成功读取了正文与两个 PoC。
- 机器可读记录：[OSV 详情](https://api.osv.dev/v1/vulns/GHSA-qwgh-2vcv-g2f7)。实际取得记录的 `modified` 为 `2026-08-19T19:30:06.686866977Z`，`published` 为 `2026-08-19T19:15:34Z`。
- OSV 的 affected package 使用 `block_buffer`（下划线），查询 `block-buffer`（短横线）仍命中；记录范围为 introduced `0`、fixed `0.12.1`。因此自动结果包含本项目 `0.10.4`，不以名称差异单独否定命中。
- 上游问题描述为：游标存放在块数据内，回调改写该字节后 panic，被调用方 catch 并继续使用时破坏游标约束；另一个 PoC 使用 `ReadBuffer`。
- 本机 Cargo registry 缓存的锁定 **0.10.4** `src/lib.rs:55–64` 中，`BlockBuffer` 明确分开保存 `buffer` 和 `pos: u8`；`digest_blocks` 在 `:138–175` 调用压缩回调前仅覆盖 `buffer`，没有把游标存放在块数据内；此版本没有 `ReadBuffer` 类型。该结构与上游 PoC 描述存在实质差异。
- 实际使用已构建的 `block-buffer 0.10.4` rlib，独立 `rustc` 编译并运行等价 Eager PoC（没有调用 Cargo 或修改仓库）：`BlockBuffer::<U4,Eager>::new(&[1,2])`，`catch_unwind` 包住 `digest_blocks(&[3,255], |_| panic!("fixture panic"))`，随后 `assert_eq!(buffer.get_pos(), 2)` **通过**。panic hook 的固定 fixture 消息为预期；退出成功。首次临时 PoC 使用了错误 re-export 路径，编译失败；改为 `block_buffer::generic_array::typenum::U4` 后才执行成功。
- 核对包哈希路径与管理摘要调用未发现捕获摘要内部 panic 后继续复用同一缓冲的代码。

**结论：保留一项扫描命中，已做源码与等价运行复核；所描述的内联游标损坏在锁定 0.10.4 未复现，尚无证据认定会影响 TwinTail 包完整性校验。** 这不是上游正式撤销或完整形式证明。公开发行前需记录负责人对范围矛盾的处置，优先向上游/数据库核实修正范围；若选择升级，先确认可用版本、依赖链与实际 API/协议回归，不直接把 `block-buffer` 强制跨大版本替换到 `digest 0.10` 下。不得隐去此命中，也不得仅据宽范围命中宣称当前可被远程利用。

复核限制：本次未运行 Miri，未覆盖所有回调/版本；直接用 Python 获取 GitHub raw/API 时本机出现 CA 验证失败，未关闭 TLS 验证。官方 GHSA 正文由网页工具成功获取；历史源码比对依据本地锁定版本 registry 缓存，未声称成功下载对应 Git tag。

### npm 锁文件

实际在 `web/` 执行：

```sh
npm audit --json
```

结果：`vulnerabilities` 为空；info/low/moderate/high/critical/total 均为 **0**。报告统计 prod 26、dev 196、optional 55、total 221（类别有重叠，不应求和）。包含开发依赖，未省略 dev。输出另有 npm 提示：`.npmrc allow-scripts` 被 package.json `allowScripts` 覆盖；这不是漏洞命中，本次没有运行安装脚本。零报告不等于依赖没有未知漏洞。

## 版本、许可证与安全政策门槛

本次逐一读取根 workspace、9 个 Rust package 的 Cargo.toml、4 个插件清单、web/package.json 与 lock 根记录：版本均为 **0.1.0**；Rust 各包 `publish = false`（直接或继承），前端为 private。wire/config/protocol 的独立版本不要求等于 package SemVer，不作混淆。

初始审计快照未发现根 LICENSE/COPYING/NOTICE/SECURITY 文件，Cargo package/workspace 和前端 package 未声明 license。**公开发行仍被项目授权条款及安全报告政策门槛阻挡**：应由项目负责人决定许可证并补齐原文、各 package 元数据及分发义务；安全政策应说明受支持范围、报告入口与处理流程。审查不替用户选择许可证，不把 `publish=false` 当作候选二进制可以公开分发的授权。本文的缺失结论仅针对上述初始快照。最终复核时文档 owner 已新增根 SECURITY.md，说明候选阶段支持范围、报告所需脱敏证据、运行限制和公开发行门槛；但尚未公布专用私密报告渠道，明确要求先通过已确认私密渠道或托管平台 private vulnerability reporting 联系。LICENSE/COPYING 与 Cargo license 元数据仍未出现，授权门槛未解除。

本轮未执行完整 SPDX/传递依赖许可证清单审计；不能据 npm 零漏洞或 OSV 请求成功宣称依赖许可证兼容。发行工具如收集第三方元数据，应复核实际落入制品的文件与义务。

## 候选工作流与后续复审

静态核对 `.github/workflows/release-candidate.yml`：Linux x64、Linux arm64、Windows x64 使用对应 native runner，`build --target` 与 stage 的 target triple 一致，native `cargo test --workspace --locked` 不带 `--target` 与其 native ABI 一致。workflow 仅 workflow_dispatch、contents read、upload-artifact，不 tag、不 publish；静态配置不构成这些 runner 已实际通过的证据。

另实际执行 `python3 -m unittest discover -s tools/state -p 'test_state.py' -v` 与 release 对应命令：state **16 项通过**、release **17 项通过**。覆盖链接/特殊文件/恶意路径、目标不覆盖、失败部分写清理、socket 占位、归档完整性和目标架构等；均为合成测试，不替代实际跨平台候选运行。reverse transport 静态核对 loopback 绑定、精确路径/单一身份和鉴权 header、活动连接期间拒绝重复 TCP、握手取消/超时、任务停止及不重放命令，未新增确认缺陷；真实反向 SnowLuma 兼容未在此次审查运行。本文为审计记录，不批准外部发布。

变更记录由 lead 合并至本轮 CHANGELOG；本审查只写本文件，不与其他文档 owner 争用。


## 审计后授权更新（2026-10-29）

用户随后明确决定项目使用MIT，并授权新建公开GitHub仓库、main分支及0.1.0发行准备。上文许可缺失和未批准外发的结论保留为初始审计快照，不能再作为当前用户授权未定的描述。根LICENSE、Cargo元数据及SECURITY由lead同步，候选工具license_gate改为MIT；发行须将许可文件纳入最终制品并重新校验。本文不宣称GitHub推送或Release已完成，也不将MIT决定扩展为第三方依赖许可兼容审计通过。

最终工具回归为state16项、release18项；上文release17项是审查时运行的历史快照。Windows MSVC/arm64目标运行与真实SnowLuma reverse仍未验证。

发行准备补充：第三方许可库存已覆盖206个Rust锁定版本与25个Web生产安装路径，全部条目附原文；本地缺失的12项文本已从精确版本归档（按Cargo.lock校验）或发布包固定VCS commit补齐。正文见根 THIRD_PARTY_NOTICES.md 与 third-party-licenses/。此为许可材料补齐证据，不扩展前述漏洞审计覆盖范围。
