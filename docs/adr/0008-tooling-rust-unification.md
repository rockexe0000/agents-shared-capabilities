---
title: "ADR 0008:能力投影 tooling 統一為單一可攜 binary(Rust)+ ephemeral-init catalog + on-pod render"
---

> Vendored 設計快照:本 ADR 由團隊內部設計記錄複製而來,作為本 repo 的自足設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)已移除或改為本地 `docs/adr/` 連結。

# ADR 0008:能力投影 tooling 統一為單一可攜 binary(Rust)+ ephemeral-init catalog + on-pod render

**Status**:Accepted · **決策日**:2026-09-13 · **Accepted**:2026-09-14(owner z)

## Context

ADR 0002/0005/0006/0007 疊出了四軸能力投影(skills / MCP / hooks / bin / 授權)。實作**分兩套 backend**:

- **dev + node image(claudecode1)**:node 跑 `agents-shared-capabilities/tools/*.js`(`sync.js`、`mcp-apply.js`)。
- **debian image(antigravity1,無 node)**:靠一疊 POSIX sh apply(`bin-apply.sh` / `skills-apply.sh` / `perms-apply.sh`)。

為了讓 no-node pod 消費,走 **option-C**:off-pod `sync.js --render` 把產物算好(`runtime-mcp.json` / `bin-install.tsv` / `skills.tar.b64` / `authz-*.json`)→ commit 進 overlay → bake 成 configMap → pre_boot 用 sh apply。pod **刻意不 clone catalog**(供應鏈邊界),所以 render 只能 off-pod。

痛點:

1. **兩套 backend 重複邏輯、易漂移。** sh 端最脆的是「無 jq 的 JSON merge」與「解析帶 frontmatter 的 YAML」——`perms-apply.sh` 的 JSON merge 甚至得靠 pod 上剛好有 `python3`/`jq`(見 ADR 0007 首驗前置)。
2. **committed 中間產物的手續。** `permissions.md` / `capabilities.md` 一改,就要重 `--render` + 重 commit `authz-*.json` 等。
3. **debian 無 node** 把 render 鎖在 off-pod。

**觸發**:owner z 在授權投影(ADR 0007)的討論串提出兩個改動——(a) `tools/` 直接改 **Rust** 重構成單一 binary(無 runtime 依賴,跑到哪都行);(b) catalog 用 **ephemeral initContainer** 整包拉、跑完釋放——合起來即「**on-pod render** 取代 render→commit→apply、消掉 committed 中間檔」。

## At a Glance

```
現況(option-C:off-pod render → commit → configMap → sh/node apply):
  agents-shared-capabilities (catalog) ──┐
  agent capabilities.md / permissions.md ┤ sync.js --render (off-pod, node)
                                         ▼
   commit 進 overlay → bake configMap → pre_boot: {bin,skills,perms}-apply.sh (debian, sh)
                                                    mcp-apply.js (claude, node)

提案(單一 Rust binary + ephemeral init 供 catalog + on-pod render):
  ephemeral initContainer:
    clone catalog @pinned-ref  +  取 pinned rust-binary
      → binary render (on-pod) 進 data PVC
      → init 結束、資源釋放(running container 不帶 catalog / 無 runtime dep)
  主容器啟動,直接吃 PVC 上剛 render 好的 config
  護欄:授權面 emit 最終 allowlist 供稽核;secret(op/vault)軸維持 off-pod
```

## Approaches Considered

### 方案 A(採用,分階段):單一 Rust binary + ephemeral-init catalog + on-pod render

- **Pros**:消滅 node/sh 兩 backend 分裂;殺掉脆弱 sh(JSON merge、YAML 解析);static binary 無 runtime 依賴,dev/claude/debian 同一支;免 committed 中間產物的重 render 手續;`ephemeral init` 讓「pod 不*帶著* catalog 跑」的邊界維持不變。
- **Cons**:重寫一套**剛穩定、又是安全路徑(供應鏈 + 授權)**的 codebase;binary 自己變成受管供應鏈物件(build / attest / pin / cross-compile);boot path 多一次網路依賴(clone + 取 binary);**授權面失去「diff 裡看得到確切 allowlist」**;帶 secret 的 MCP 軸要小心。

### 方案 B:維持現狀(node + sh 兩 backend + committed 產物)

- **Pros**:已運作;授權面 committed 產物在 PR diff 可 review;configMap-baked → git 掛了也開得起來。
- **Cons**:兩套維護、sh 脆、每上一軸多一份 sh + 一份重 render 手續。

### 方案 C:on-pod render 但沿用 node

- **否決**:debian image 無 node;要嘛在 debian 裝 node(違背 no-node 初衷),要嘛另做 node init image——都比一支 static binary 重。

### 外部先例

GitOps 兩派並存:**render off-pod + commit**(Kustomize/Helm template 出 manifest)vs **operator/init 動態 render**。用**單一 static binary(Rust/Go)**做 config 投影以避免 runtime 依賴,是常見且成熟的作法;ephemeral init 拉資料、render、退出,也是標準 pattern(如 init 拉 secret/config)。

## Decisions(Accepted 2026-09-14)

1. **`tools/` 重寫為單一 runtime-portable Rust binary。** 對外 CLI 與現行 `sync.js` 對齊(`sync` / `--render` / `--check` / `--with-tools` / `--check-tools`);逐軸與 node 做 parity,全等前不拔 node/sh。
2. **binary 視為受管供應鏈物件。** cross-compile(linux amd64/arm64 + macos)、release 後 sha256 pin + verify、build/attest 流程明訂——把 ADR 0006 對「外部 bin」的那套,套在**自家工具**上。
3. **catalog 以 ephemeral initContainer 供給。** clone **pinned ref**(非浮動 branch)→ 跑 binary render 進 data PVC → init 結束釋放;running container 不帶 catalog(邊界維持)。
4. **on-pod render 取代 render→commit→apply**,並以移除 committed 中間產物(`authz-*.json` / `bin-install.tsv` / `skills.tar.b64` / `runtime-mcp.json`)為終態——但受 5/6 護欄約束。
5. **授權面護欄(deploy-time 可稽核 + 決定性)。** 結果由 `catalog pinned-ref` + `permissions.md ref` 唯一決定(deterministic);render 後把**最終 allowlist emit 一份到 log/PVC 供稽核**。若「emit 稽核」不足以取代「diff 裡看得到」,**授權軸最後才切、或保留 committed 產物**——授權面寧可回溯 > 省一個檔。
6. **secret 護欄。** `env:` 參照本就 pod 端解(k8s secret 注入),搬 on-pod 無新增曝險;**pull 型 backend(`op://` / `vault:`)維持 off-pod 或只讓 init 碰 ref、不碰原始值**,不破 facade「cred 不落 agent」。帶 secret 的 MCP 軸視情況保留 off-pod render。
7. **分階段、平行不斷線。** binary 先與既有 sh/node 並存 → 逐軸 parity(CI 比對 rust 產物 == node 產物)→ 一軸一軸切 on-pod(**先 authz**:無 secret、blast radius 最小)→ 全數驗畢再拔 sh + `mcp-apply.js` + committed 產物。

## Lessons learned(可重用,踩過的雷)

- **Phase 0(authz `--render` spike,2026-09-14):** parity 用「輸出端對齊」最省事——Rust 直接產 `JSON.stringify(obj,null,2)+"\n"` 等價字串(空陣列 inline、非空每列縮排、C0 escape),不引 `serde_json`(其 escaping/key-order 還得回頭比對 V8)。authz 軸零外部 crate 就夠。sort 依 JS 預設(UTF-16 code unit),ASCII 指令名與 byte order 同,但仍照 UTF-16 比以求全等。
- **本機無 C linker 貫穿全程(Phase 0→4):** 撰寫環境無 `cc`/gcc、非 root,只能 `cargo check`/`fmt`/`clippy` + `parity.sh --node-only`;**rust==node byte 比對一律靠 CI**(ubuntu-latest)。代價很實在:Phase 4 的 `apply_bin` 有個 bug(建 `BinTool` 時 `Platform.asset` 給空字串 → `install_tool` 的 `tmp.join(&asset)` 解析成暫存**目錄** → `curl -o <dir>` 失敗 → apply 回非零),**本機四個 parity 測試全過、只有 CI 抓到**(且 harness 用 `>/dev/null` 吞了輸出,只看到停在某軸)。教訓:(a) 執行面的東西別以為本機綠就沒事;(b) parity harness 的 rust 呼叫要 `|| { echo "<axis> errored"; fail=1; }`,別讓裸 exit 蓋掉後面。
- **parity 面分兩類:** `--render` 吐**定形檔**可逐 byte diff(golden);live `sync`/`apply` **改 HOME 狀態**,得隔離 HOME 跑 node/rust 兩邊、比投影結果(settings.json / mcp.json / 裝出的 binary / skills 樹)。後者面較大、擺後段。
- **on-pod 供給的 catalog 邊界(Phase 4 關鍵前提修正):** 一度以為「`capsync sync` 取代 pod appliers」即可,查證才發現 **sync 的 skill 投影是 symlink 到 catalog checkout**、需 catalog 常駐 → 破 Decision 3「running pod 不帶 catalog」;且 **sync 不做 authz**。改為新增 `capsync apply` 吃 `--render` 的**自足產物**(tar bundle / tsv / 最終 JSON),**不碰 catalog** → 邊界保住。教訓:render/apply 分工的價值就在「apply 不需要 catalog」,別讓 apply 回頭依賴它。
- **capsync 必須 staged 到 PVC:** pre_boot 在**主容器**跑 `capsync apply`,但抓 binary 的 `cap-render` **init** 的 `/tmp` 開完就釋放 → 得把驗過的 binary `cp` 到 `$HOME/bin`(data PVC、跨容器共享、on-PATH)主容器才拿得到。init 抓的東西要留給主容器,一律走 PVC。
- **on-pod render 的 fail-closed(authz 面)實測夠用:** render/verify/authz 產物任一失敗 → init 非零退出**擋開機**,不 fallback、不用過期 allowlist;fetch/clone 先 retry 3 次擋抖動。可用性代價有界——pod 開機**本就硬依賴 GitHub**(memory-bootstrap clone memory repos),此舉只多 capsync release + catalog 兩個同類依賴,**沒增新類**。負向測(壞 digest→擋開機)owner 選擇未實跑,code path 明確。
- **授權稽核可見性(Decision 5 定案):** 拔 committed authz 後,allowlist 不再在 agents-infra 的 PR diff 可見;稽核面改「pod log 的 allowlist emit + cold `permissions.md` 的 PR review」。owner 認可即接受此取捨——**但務必在拔除的 PR 明列此代價**,別讓「diff 看得到」這個護欄無聲消失。
- **`capsync apply` 各軸必須獨立:** 初版用 `?` 逐軸傳播,早軸(尤其 bin **連網**)失敗會**跳過後面的 authz**(安全軸)。改各軸獨立跑、**authz 先、bin 最後**,單軸失敗只記錄不中止。單一 binary 把多支 applier 併成一個 process 時,這種「一顆老鼠屎」風險是新的,要主動設計掉。
- **供應鏈治理套自家 binary(Phase 1g):** toolchain 由浮動 `stable` 釘死精確版(可重現);release on `v*` tag → cross-compile static **musl** linux(amd64/arm64,同一支跑 node image 與 debian image)+ macos,每 artifact sha256 + build-provenance attestation。**repo-wide `v*` 版號**(capsync 是本 repo 唯一發版物),binary↔同 commit catalog schema 綁定,緩解版本 skew。消費端(cap-render)pin `CAPSYNC_VERSION` + 兩 digest + `CATALOG_REF` 三者一起 bump。
- **secret 邊界(Decision 6 落地):** render 產物只帶 `${env:}` **參照**、不解真值(facade/openab runtime 才從 pod env 解),故 MCP 軸搬 on-pod 零新增曝險。順手退掉 dotenv-file 後備(`secrets/.env`)——`env:`/`${VAR}` 只認 `process.env`;`op://`/`vault:`/`keychain:` 本就走各自 CLI、不經此檔。

## Rejected alternatives

- **on-pod render 用 node** —— debian 無 node;等於在 debian 裝 node,違 no-node 初衷。
- **catalog 常駐 running pod** —— 破供應鏈邊界 + 佔資源;改 ephemeral init。
- **一次性大爆改** —— 對剛穩定的安全路徑風險過高;改分階段、平行遷移。
- **維持現狀** —— 兩 backend 維護成本 + sh 脆弱面 + 重 render 手續是持續稅,值得一次收掉。

## Consequences

- **得到**:單一 backend、無 runtime 依賴、免重 render 手續、sh 脆弱面(JSON/YAML)消失、pod 跑什麼 image 都同一支工具。
- **代價 / 待償**:Rust 重寫工時;自家工具納入供應鏈治理(build/attest/pin);boot path 多網路依賴;需補「授權面稽核可見性」;secret 軸邊界要主動守。
- **前提**:ADR 0007 授權投影**已落地**(2026-09-13 antigravity1 實機驗通、handoff stage 已交付;本 ADR 不阻擋、亦不取代它);遷移期間 rust 與 node/sh 兩套並存,以 parity gate 保護。

## 相關 ADR

- [ADR 0002](0002-agent-capability-provisioning.md) — 能力供給母 ADR(四軸 + option-C 由來)。
- [ADR 0006](0006-binary-dependency-provisioning.md) — binary 供給(pin/verify 樣板,套用到自家 binary)。
- [ADR 0007](0007-authorization-projection.md) — 授權投影(首個要維持護欄的軸,已落地)。
