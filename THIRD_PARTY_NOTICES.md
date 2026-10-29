# Third-party notices

TwinTail's own code is licensed under the MIT License in [LICENSE](LICENSE). That choice does not relicense third-party software. Each dependency remains subject to its own license, copyright notices and applicable additional terms.

Distribute this file together with both consolidated text files:

- [Rust dependency license texts](third-party-licenses/rust-dependencies.txt)
- [Web dependency license texts](third-party-licenses/web-dependencies.txt)

The Rust inventory contains all **206 third-party package versions in Cargo.lock**. It deliberately includes development/build dependencies, optional packages and packages for other targets, so it is a conservative superset of the production dependency graph of `twintail-host`, `adapter-snowluma`, `echo-plugin`, `mock-adapter` and `web-admin`. It is not a claim that every listed package is linked into every binary.

The Web inventory contains all **25 production installation paths** marked non-development in `web/package-lock.json`, recursively covering Vue and its production dependencies, including the TypeScript peer. Duplicate installed versions/paths are retained; build/test-only tooling is excluded. Bundling may remove portions of this superset.

Package versions, upstream license expressions, registry/download sources, original license texts, and available copyright/author notices are included in the consolidated files. Source file labels refer to paths inside the published dependency package. Unicode license texts, ring's BoringSSL/other-bits/Fiat and other bundled notices, and TypeScript's third-party notices are preserved. The initial inventory was collected locally; the supplementary source collection below completed the twelve original-text gaps using verified-TLS upstream downloads.

For an upstream alternative such as `MIT OR Apache-2.0`, TwinTail elects MIT when available; otherwise it elects Apache-2.0 when offered. Legacy slash-separated alternatives are preserved as published. This election does not remove additional conjunctive requirements (for example Unicode-3.0, ISC or BSD notices), and does not apply to dependencies offered only under another license. All discovered legal files are retained even when they describe an unselected alternative. In particular, the `r-efi` AUTHORS files contain the complete elected MIT text and copyright notices; their alternative LGPL statements do not change this MIT election.

## Collection provenance and completeness

The Rust inventory preserves original legal text for all **206 third-party package versions in Cargo.lock**. The initial offline Linux metadata/cache collection omitted twelve package versions; those original-text gaps have now been completed from their exact upstream sources:

- `uuid-simd 0.8.0` and `vsimd 0.8.0`: both published `.cargo_vcs_info.json` files pin Nugine/simd commit `d74c030d9dc4f3cae02146d1f497ff62726ef09a`. Their archives omit the repository-root LICENSE. The original MIT text, including `Copyright (c) 2021 Nugine`, was obtained through the official GitHub API at that exact commit, reproduced separately in each entry, and labeled as an upstream-root source rather than a file present in the crate archive. Each entry includes the immutable source URL and the retrieved text SHA256.
- `windows-sys 0.52.0`, `windows-targets 0.52.6` and the eight locked `windows_* 0.52.6` architecture packages: the exact published archives were downloaded from official `static.crates.io` URLs. Every archive SHA256 matched its Cargo.lock checksum before original `license-mit` and `license-apache-2.0` files were read and reproduced. Each entry records the archive URL and its own published VCS commit/path; no text from another version was substituted.

Supplementary downloads retained TLS certificate verification. The inventory retains both Windows license alternatives for preservation while applying the MIT election described above. All twelve entries now contain full original text, rather than only SPDX identifiers or unresolved placeholders. This completes the stated lockfile inventory; it does not certify packages or bundled content outside that scope. Updating dependencies requires refreshing and revalidating these inventories.
