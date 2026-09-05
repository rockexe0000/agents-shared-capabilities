---
name: hello-capability
description: Minimal example skill proving the capability-provisioning pipeline end to end. Use only as a scaffold reference when authoring a real skill; it prints a short confirmation and does nothing else.
metadata:
  short-description: Example scaffold skill
capability:
  tools: []
  network: false
source: agents-shared-capabilities (built-in example)
pinned-ref: n/a
checksum: n/a
---

# Hello Capability

這是一個**範例 skill**,用來驗證 catalog → `sync` → runtime 的投影管線可運作。

真正撰寫 skill 時:

- `description` 用第三人稱、同時寫「做什麼 + 何時用」+ 關鍵詞(觸發命中的關鍵)。
- 需要腳本放 `scripts/`,由 bash 執行、程式碼不進 context;細節參考檔放 `references/`。
- `capability` frontmatter 宣告足跡(要動的工具、是否連網)——這是**宣告**,真正授權在 `permissions.md`。
- 外部來源的 skill 要填 `pinned-ref` 與 `checksum`,並經 PR 審核 vendored 進來。

## 步驟

1. 確認被 `~/personal/capabilities.md` 啟用。
2. 回報 `hello-capability: pipeline OK`。
