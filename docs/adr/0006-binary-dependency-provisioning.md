---
title: "ADR 0006:Skill 的 binary/CLI 依賴供給(第四軸,釘版本安裝 + 三層自檢 + 引擎/內容分離)"
---

> Vendored 設計快照:本 ADR 由團隊內部設計記錄複製而來,作為本 repo 的自足設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)已移除或改為本地 `docs/adr/` 連結。

# ADR 0006:Skill 的 binary/CLI 依賴供給(第四軸,釘版本安裝 + 三層自檢 + 引擎/內容分離)

**Status**:Accepted · **決策日**:2026-09-08 · **延伸**:ADR 0002

## Context

把 `github.com/oablab/cfdrop` 的 skill 接進 catalog(ADR 0002 機制)後暴露一個缺口:`cfdrop` skill 會 **shell out 到一顆 native binary**(自帶的 Rust CLI),但 catalog 目前**只管指令與設定**——三條既有供給軸(`skills/` 的 `SKILL.md`、`mcp/registry.yaml`、`hooks/registry.yaml`)與 `sync.js`(symlink skill 目錄 + 投影 MCP/hook config)**都沒有涵蓋 skill 依賴的執行檔本身**。

現況缺口:
- skill 能不能真的跑,取決於機器 PATH 上有沒有那顆 binary;現在的答案是「請自己去 GitHub releases 抓」= **隱性手動安裝**。
- 手動安裝 = **跨機器版本漂移**(dev/pod/各 agent 版本不一,skill 的 feature-floor 假設會靜默失效)+ **未釘版本、未 checksum 的不透明第三方執行檔**——這是供應鏈裡最尖的一種暴露面,比 `SKILL.md`(純文字)或 MCP server(走 facade、憑證留閘道側)都更危險。

判準(承接 ADR 0002):
- binary 依賴是 catalog 的**第四條資料軸**,和 skill/mcp/hook 同構(「host-agnostic 宣告 → 投影/安裝」),不另立體系。
- 兩軸切分延續:`capabilities.md`(enable)決定裝哪些、`permissions.md` / Hot Permission Boundary(authorize)決定准不准跑;binary 只是把「裝」從「複製檔案」擴張到「取得可執行檔」。
- 因 binary 是最不透明的第三方碼,其**授權層級要 ≥ skill/MCP**:釘版本 + checksum 必備,安裝/執行過 `ask` 供應鏈閘。
- 這套一旦成形,`bin/` 軸與安裝器會是「把 repo 當範本給別人用」的表面之一,故**從第一天就要 org/host/runtime 中立**。

## At a Glance

```
  agents-shared-capabilities/            (集體 catalog;資料軸皆 top-level 兄弟)
    skills/<name>/SKILL.md               requires: [{name: cfdrop, min: 0.4.0}]   ← skill 宣告依賴
    mcp/registry.yaml
    hooks/registry.yaml
    bin/registry.yaml   ★ 新第四軸       name→ source / pinned-version / per-(os,arch) sha256 [+ provenance]
    tools/               ← 維運機器(sync/lint/projectors);不放資料軸
         │
    capabilities.md(enable) ──┐
         │  sync 算閉包:enabled skills → 其 requires → 釘版 manifest
         ▼                    │
   ┌─────────────────────┐    │    ┌──────────────────────────────┐
   │ dev:sync 安裝器      │    │    │ pod:agents-infra build-time  │
   │ --with-tools(opt-in)│    │    │ 讀同一 manifest 烤進 image     │
   │ 抓+驗 sha256 → 受管   │    │    │ (render→image;pod 不 clone)  │
   │ bin dir(掛 PATH)    │    │    └──────────────────────────────┘
   └─────────────────────┘    │
         │                    ▼
   三層自檢(共用 manifest checksum):
     (1) 裝前 already-satisfied skip   (2) sync --check 漂移(PATH/版本/checksum,exit 1)   (3) skill runtime preflight(feature-floor,fail loud)
         │
    執行外部 binary 一律過 Hot Permission Boundary(install external tool = ask)
```

## Approaches Considered

### 外部先例對照(取得釘版本 release 執行檔)

| 手法 | 定位 | 可借 / 要避 |
| --- | --- | --- |
| `cargo-binstall` | 從 crates/GitHub releases 取 Rust prebuilt binary | cfdrop 正是 Rust；可當後端之一,但綁 Rust 生態 |
| `ubi` / `eget` | 通用「抓某 GitHub release 的對的平台 asset」 | 正中需求:平台選擇 + 釘 tag;我們只需在外面包 checksum/provenance 驗證 |
| `asdf` / `mise` | 多語言版本管理器,plugin 生態 | 重、plugin 供應鏈自身要信;對「少數幾顆釘版 binary」殺雞用牛刀 |
| `gh release download` | 最小可用,已在環境裡 | 零依賴 fallback;要自己做平台選擇 + checksum |
| Nix | 完全可重現 | 最強保證但導入成本/心智負擔高,與現有輕量投影模型不搭 |

**讀法**:「抓對平台的釘版 release asset」是**已被解決的 plumbing**(ubi/eget/cargo-binstall/`gh release download`)。我們**不自建 package manager**,只掌握真正的價值與風險點——**manifest(釘版 + per-platform checksum + provenance)與安裝前後的驗證**——把抓取委給現成 fetcher。

### 設計岔路(前期已與 owner 逐一裁決)

- **裝哪**:受管專屬 bin dir(可逆、免 root)vs 系統/host 套件管理器 vs skill 自帶。→ 受管。
- **checksum 維護 / bump**:純手動 vs 半自動工具算+人審 PR vs 全自動追 latest。→ 半自動 + 人審;新版偵測只通知。
- **provenance**:只 sha256 vs sha256+有就驗attestation/降級標記 vs 強制簽章才准進。→ 中間案(有就驗、無則顯性降級)。
- **安裝時機**:sync 預設裝 vs opt-in flag vs 分 host 預設。→ opt-in 起步 → host-aware 預設。
- **pod**:同批做 vs dev 先行 pod follow-up vs 只做 pod。→ dev 先行。

## Decisions

1. **新增第四資料軸 `bin/`,不寄生在 `tools/`。** `bin/registry.yaml` 是 binary 依賴的集體宣告,top-level 兄弟於 `skills/`、`mcp/`、`hooks/`;`tools/` 永遠只放維運機器(sync/lint/projectors)——維持「宣告資料 vs 執行碼」分離。命名去撞:repo 內 `bin/registry.yaml` = **宣告**,host 上受管的安裝**產物**目錄另取名(如 `state/bin`)以免同字混淆。
2. **skill 宣告、registry 定義並釘版。** `SKILL.md` 加 `requires: [{name, min}]`(依賴跟著 skill 走、隨 vendoring 一起帶);`bin/registry.yaml` 把每顆 binary 定義一次:`name` / `source`(release repo)/ `pinned-version` / 每個 `(os,arch)` 的 asset + `sha256`〔+ 選用 provenance〕。**版本地板 vs 釘版**:skill 宣告下限(對映 cfdrop 既有 feature-floor:`--md`≥0.2.0、mermaid≥0.4.0…),registry 釘實際版本;`lint` 守「釘版 ≥ 每個要求它的 skill 的 floor」且 `requires` 可 resolve 到 registry(複用 `sync` 現有 skill/server name 交叉檢查)。
3. **受管、可逆的安裝位置。** binary 裝進受管 bin dir、由安裝器掛 PATH,disable skill 後其獨佔 binary 可被清除(GC)。不污染系統、不需 root、版本由 catalog 管而非 host 全域——避免多 agent / 多版本互撞,並讓漂移可確定性比對。
4. **單一宣告、多安裝後端。** dev/host = **sync-time 安裝器**(`sync.js --with-tools`,opt-in 起步;穩定後 host-aware 預設);pod = **agents-infra build-time 烤進 image**,讀**同一份 manifest**,契合既有 render→configMap、pod 不 clone/不 on-pod sync 的哲學;deploy-time init 安裝只當沒有 build pipeline 的 escape hatch。某 agent 要哪些 binary 由 `enabled skills → required bins → 釘版 manifest` **推導**(與現有 render 同邏輯)。
5. **三層自檢,共用 manifest 的 per-platform checksum 為真相來源。** (1)**裝前 already-satisfied skip**:比對 `name@pinned` 是否已在且 checksum 相符,是則跳過、壞則重裝或報錯——手動裝過的不被重裝、裝錯的被抓到;(2)**`sync.js --check` 漂移模式**(延伸既有 `--check`):驗每個 enabled skill 的 required binary 在 PATH、版本、checksum,是否相符,不符 exit 1 供 CI / 维运 cron;(3)**skill runtime preflight**:skill 自身對 feature-floor 做版本檢查(cfdrop SKILL.md 既有樣板)並 fail loud——最後一道,擋「有裝但非受管/版本不對」。
6. **供應鏈閘 + 驗證分級。** 安裝/執行外部 binary = **`ask`**:把 `permissions.md` 既有「install external skill」延伸為「install external tool/binary」,授權層級 ≥ skill/MCP。驗證:**sha256 打底**(pin + 雜湊),**有 GitHub artifact attestation / cosign 就驗**、沒有則降級但在 registry 顯性標 `provenance: none`(review 看得到,而非隱形);**不強制**簽章才准進(否則擋掉尚無簽章的小工具)。**版本 bump**由人發起、`tools/refresh-*` 抓各平台 asset 算 checksum 寫回、PR review 落地;上游新版偵測**只通知、不自動 bump**(自動拉不透明碼進來違背 ask 閘)。
7. **引擎/內容分離,為「repo 當範本」鋪路。** `bin/registry.yaml` 格式與安裝器**從第一天設計成 runtime/host/org 中立**。把 OAB 專屬耦合(`route: facade` 的 OAB MCP Facade、octobroker、`agent-bot/{uid}` cold 命名空間、member-directory/permissions overlay)以 plugin/seam 抽出,使 **引擎**(`tools/`、`templates/`、schema、projectors、各 registry 的**格式**)可被他人 clone 當範本、**自帶內容**(自己的 `skills/`、`mcp`、`bin` entries)。前提認知:**projectors 是護城河也是維護負擔**(編碼各 runtime 的 config 格式),範本壽命 = 讓 projectors 跟上 runtime 變化。

## Lessons learned(可重用,踩過的雷)

（決策階段尚未實作;PoC / 首次落地後回填。首個驗證對象 = **cfdrop**:Rust binary、releases 已有 linux-amd64/arm64 + macos-arm64 prebuilt,適合驗 manifest + fetcher + checksum;因是自家 `oablab`,可請上游開 **GitHub artifact attestation**,provenance 免費到手。待回填:受管 bin dir 與各 runtime PATH 注入的實際邊界、ubi/eget/cargo-binstall 當後端的取捨、`--check` 漂移在 pod 烤 image 情境下對什麼比對、bump refresh 工具的平台矩陣維護成本。）

## Rejected alternatives

- **把 binary 塞進 `tools/`**——混淆「維運機器」與「資料軸」;改設 top-level `bin/`。
- **自建 package manager / 下載器**——重造輪子;改包現成釘版 release fetcher,只自己掌握 manifest + 驗證。
- **系統/全域安裝(`/usr/local/bin`、brew/apt)**——污染系統、需權限、跨 distro 不一致、難漂移檢查。
- **enable 觸發時靜默抓+跑 binary / sync 預設就裝**——把原本安全的「投影設定」偷渡成「跑安裝器」;改 opt-in + ask 閘。
- **全自動追 upstream latest 自動 bump**——把不透明執行碼自動拉進來,違 ask 閘;改人審 bump、機器只算 checksum。
- **強制 provenance/簽章才准進 catalog**——太硬,擋掉尚無簽章的可用小工具;改「有就驗、無則顯性降級」。
- **pod 每次啟動 init 裝 binary**——不可變性差、runtime 連網;改 build-time 烤 image,init 只當 escape hatch。

## Consequences

- skill 不再有**隱性 binary 依賴**:dev 一鍵 `sync --with-tools`、pod 走 render→pre_boot `bin-apply`(見 **Addendum:Phase D**,非原文的 image bake),兩者讀**同一份釘版 manifest** → 跨 host 不漂移。
- `sync` 取得「連網下載 + 落地可執行檔」的新能力 = 新 blast radius;以 **opt-in + `ask` 閘 + checksum + preflight** 四重收斂。
- 新增維護面:per-platform checksum 矩陣、`bin/registry.yaml`、sync 安裝器、`tools/refresh-*`、lint 的 floor≤釘版檢查、（pod）image bake 步驟。
- 為「`agents-shared-capabilities` 當範本給別人管自己 agent 的 capabilities」開路,但其壽命綁 projectors 是否跟上各 runtime。
- **待辦(採行時另開 / 見對應 handoff,stage 規劃/實作中)**:`bin/` 軸 + `bin` schema(warm)、`SKILL.md` `requires` 欄與 template、`lint.js` 擴充(floor / resolve / 外部需 pinned+checksum)、`sync.js` 安裝器 + `--check` bin 模式、`permissions.md` 新規則「install external tool」、`tools/refresh-*`、agents-infra pod bake(**D-render**,見 Addendum,非 image bake)、範本化 seam 抽離。先行範圍:dev 安裝器 + cfdrop 為首個驗證對象。

## Addendum:Phase D 落地為 D-render,非 image bake(2026-09-08)

實作 Phase D(pod)時發現 **Decision 4 的前提有誤**:D4 寫「pod = agents-infra build-time 烤進 image」,但 **agents-infra 是純 kustomize/GitOps repo、不 build image**;agent image 是外部的 `ghcr.io/openabdev/openab`(按 digest 釘,建於 `openabdev/openab`)。故「build-time image bake」若要做,得動上游 image repo,且會失去 per-agent 顆粒度(image 全 agent 共用)。

**修正決策(owner 2026-09-08 拍板 D-render)**:pod 走與 MCP **同一條 option-C** —— off-pod `sync.js --render` 產 `bin-manifest.json`(該 agent required tools 子集,帶全平台)→ commit 進 overlay → configMap 掛 `/etc/openab/mcp` → **pre_boot `bin-apply.js`** 解析 pod 平台、抓+驗 sha256、裝進持久 HOME。

**理由**:① 與 agents-infra 既有機制(render→configMap→pre_boot,見 mcp-apply.js)一致;② **per-agent**(由各 agent `capabilities.md` 驅動,image bake 做不到);③ 不需動上游 image repo;④ HOME 在資料 PVC → 裝一次、跨重啟保留,近似 bake。原 D4 稱此為「escape hatch」,但在此 repo 現況下它才是正解。true image bake 留待 `openabdev/openab` 若要提供共用底線工具時再議。

**落地**:catalog `sync.js --render` 加吐 `bin-manifest.json`(agents-shared-capabilities #16);agents-infra claudecode1 overlay 加 `bin-apply.js` + manifest + pre_boot 一行(agents-infra #40)。首個對象 cfdrop@v0.6.1,本機 linux-arm64 端到端驗過。

**Lessons learned(回填)**:pod PATH 是隱藏子問題——binary 要落在 agent tool 子程序 PATH 上的可寫目錄;bin-apply 採「PATH 上第一個 HOME-under 可寫目錄,否則 `~/.local/bin` + WARN」的自適應法。**實機驗證(claudecode1,2026-09-09)推翻了「~/.local/bin 夠用」的假設**:openab claude image 的 PATH 是 `/home/node/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/games:/usr/games`,**`~/.local/bin` 根本不在 PATH**、唯一可寫又在 PATH 的是 **`/home/node/bin`(= `$HOME/bin`)**;首 boot 時 `~/bin` 還沒建、`~/.local/bin` 已存在但不在 PATH,所以自適應法誤落 `~/.local/bin`,cfdrop 裝了卻叫不到。修法:pod overlay 的 pre_boot 明確 `export BIN_INSTALL_DIR="$HOME/bin"`(agents-infra #44);bin-apply 冪等以 `lock.target` 為鍵,故會把 cfdrop 從 `~/.local/bin` 搬到 `~/bin`。**教訓**:自適應「猜 PATH 可寫目錄」不可靠(首 boot 目錄未建、image PATH 各異);per-host 用 `BIN_INSTALL_DIR` 明示才穩。其餘待回填:ubi/eget/cargo-binstall 當後端的取捨、bump refresh 工具的平台矩陣維護成本。

**安裝位置修正(Decision 3 refine,2026-09-08,owner 要求 dev/pod 統一)**:D3 原文的「受管專屬 bin dir(`state/bin`)」在 dev **永不在 PATH**(要手動 export),與 pod 的自適應選法分岔。改為 dev+pod **同一套 precedence**:① `BIN_INSTALL_DIR` 覆蓋 → ② `~/.local/bin`(在 PATH 時,慣例 user bin dir)→ ③ PATH 上既有可寫 HOME 目錄(不 regress 只有 npm-global 在 PATH 的 host)→ ④ `~/.local/bin` + WARN。**可逆性不靠「binary 放哪」而靠 lockfile**(續留 `state/bin-lock`,記錄實際 `target`),故 binary 移到 `~/.local/bin`、GC/drift 照舊。PR:catalog #18、agents-infra #42。

## 相關 ADR

- [ADR 0002](0002-agent-capability-provisioning.md) — capability 供給機制;projector/lint/供應鏈/enable-permissions 兩軸骨架。
- [ADR 0005](0005-hook-capability-provisioning.md) — 同源第三軸先例(hook 納入供給,授權前移);本 ADR 為第四軸。
