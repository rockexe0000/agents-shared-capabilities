# 貢獻指南(Contributing)

先讀 [README](README.md) 了解這個 catalog 的角色與 layout,再動手。設計與 WHY(ADR)收錄於 [`docs/adr/`](docs/adr/)。

## 核心原則

- **PR = review 閘。** 所有 catalog 變更(尤其接入外部 skill / MCP / hook / binary)一律走 PR,merge 即審核。不 live-link 遠端 marketplace。
- **憑證不進 repo。** registry 只放 secret 參照(`env:` / `op://` / `vault:`),真值放本地 `secrets/.env`(已 gitignore)。範本/測試只用假資料。
- **跟著既有 layout 走。** 集體能力放對應目錄;個人專屬能力放該 agent 的 cold 命名空間 `agent-bot/{uid}/`,不進本 repo。

## 本地開發

全部工具是 `tools-rs/` 的單一 Rust binary `capsync`(**零外部 crate**,無 node、無 npm;ADR 0008 已讓 node `tools/` + sh applier 退役)。

```sh
cd tools-rs
cargo test                               # 單元測試(parser / json / sha256 / base64 / shape)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
./parity.sh                              # build capsync + render vs committed golden 逐 byte 比對 + stateful 斷言
./parity.sh --update-golden              # 從 capsync 重生 golden(維護用)
```

catalog lint(registry / SKILL frontmatter / capabilities 引用 / 供應鏈 pin):

```sh
tools-rs/target/release/capsync lint --catalog .
```

> 無 C linker 的環境仍可跑 `cargo check` / `fmt` / `clippy`;`cargo test` / `build` / `./parity.sh` 需連結,執行期比對在 CI 跑。

## 接入新能力(supply-chain 規矩)

- **skill:** 於 `skills/<name>/SKILL.md` 加 frontmatter(`name` + `description`);要 shell out 的 native binary 用 `requires: [{name, min}]` 引用 `bin/`。
- **MCP server:** 加進 `mcp/registry.yaml`;預設 `route: facade`(agent runtime 不持 key),secret 只放 `${env:VAR}` 參照。
- **binary/CLI:** 於 `bin/registry.yaml` 宣告一次,**必須** pinned-version + 每個 `(os-arch)` 的 `asset` + `sha256`(`lint` 強制),`provenance: none|attestation|cosign`。外部來源一律 vendored + 釘版本。
- **hook:** 加進 `hooks/registry.yaml`(canonical event + command,ADR 0005);外部 hook 需 vendored + 釘版本 + checksum。

範本都在 `templates/`。

## 送 PR 前

- `capsync lint --catalog .` 綠。
- 動到 `tools-rs/` 就跑 `cargo test` + `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `./parity.sh`;render 行為有變就先 `./parity.sh --update-golden` 更新 golden 並一起 commit。
- commit message 講清楚改了什麼、為什麼;有對應 ADR / handoff 就引用。
- CI(`lint`、`tools-rs-parity`)必須全綠。
