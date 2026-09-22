# 安全政策(Security Policy)

本 repo 是跨 Coding Agent 的**能力供給正本**(capability catalog):被接進來的 skill、MCP(Model Context Protocol)server、hook 與 binary/CLI 依賴,會投影進各 agent runtime 並實際執行。因此把它當**供應鏈物件**看待——回報漏洞前請先讀本文。

## 回報管道

**請勿開公開 issue 揭露漏洞。**

- 優先走 GitHub 的 **Private vulnerability reporting**(repo → Security → Report a vulnerability)。
- 若無法使用,經由 repo owner 的私訊管道回報。

回報時附上:受影響的檔案/registry 條目、重現步驟或 PoC(Proof of Concept)、以及你評估的影響面(誤植版本 / checksum 不符 / secret 外洩 / 授權繞過等)。

我們會盡快確認收到並評估影響;修好後會在變更說明中致謝(除非你希望匿名)。

## 範圍

**在範圍內:**

- catalog 內容:`skills/`、`mcp/registry.yaml`、`hooks/registry.yaml`、`bin/registry.yaml`。
- 投影工具:`tools-rs/` 的單一 binary `capsync`(render / check / sync / apply / lint;node `tools/` 已退役)。
- 供應鏈完整性:pinned-version 缺漏、per-platform `sha256` 不符或可繞過、provenance 驗證失效。
- 授權邊界繞過:能讓 skill script / MCP tool 跳過執行期 permission 閘的路徑。
- Secret 外洩:registry 或範本內出現真實憑證(應只放 `env:` / `op://` / `vault:` 參照)。

**不在範圍內:**

- 各 agent 本地的 `~/personal/`(`capabilities.md` / `permissions.md` / secret),本 repo 不承載其內容。
- 上游第三方 skill / MCP / binary 自身的漏洞——請向其上游回報;本 repo 只負責 pin + 驗 + vendored 接入。

## 安全設計(context)

- **兩道閘:** 閘 1 存在(`capabilities.md` 決定 enable 什麼,預設全關);閘 2 授權(執行期過 Hot Permission Boundary)。載入過濾 ≠ shell 授權邊界。
- **供應鏈治理:** 外部 binary/CLI 一律 pinned-version + per-platform `sha256`(`lint` 強制),`provenance: none|attestation|cosign`;安裝/執行外部能力過 `ask` 閘。細節見 README 與 ADR `0002` / `0006` / `0008`。
- **憑證不進 repo:** registry 只放 secret 參照,真值由本地 resolver 取。
