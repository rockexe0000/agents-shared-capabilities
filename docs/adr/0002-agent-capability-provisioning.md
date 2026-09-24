---
title: "ADR 0002:Agent 能力供給機制(Skills + MCP)"
---

> Vendored 設計快照:本 ADR 由團隊內部設計記錄複製而來,作為本 repo 的自足設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)已移除或改為本地 `docs/adr/` 連結。

# ADR 0002:Agent 能力供給機制(Skills + MCP)

**Status**:Accepted · **決策日**:2026-09-04 · revised 2026-09-05(見 Addendum:MCP facade-default routing)

## Context

owner 想讓 Agent-Bot 能使用 **Agent Skills**,並設計一套「方便管理、且能被不同 Coding Agent(Claude Code、Codex、Antigravity、opencode…)共用」的機制;同一輪也要求把 **MCP(Model Context Protocol)server 的管理**併入同一機制。

現況:
- 記憶已是三層(Hot `AGENTS.md` / Warm `~/shared`、`~/personal` / Cold Hugo repo),且**與 host 無關的正本 → per-runtime 投影**是既有骨架(`AGENTS.md : CLAUDE.md/GEMINI.md`;`permissions.md` → 各 CLI enforcement config)。
- `memory-architecture` 早已把「Skills」列為 Warm 層 artifact、把「skill metadata match」列為 rule-based 觸發。
- 本機同時有 `~/.claude/skills/`(空)與 `~/.codex/skills/`(內建);兩家吃的是同一種 `SKILL.md`(frontmatter `name`+`description`)開放格式——這正是「跨 Coding Agent」的支點。
- 前一輪已有設計記錄「Skills 與 Memory 機制對位」(機制事實 + 該不該整合的分析)。本 ADR 是其後續:把「能力**供給機制**」的結構決策定下來。

判準:Skill/MCP 應**當作 Warm 層 artifact 治理,不另立體系**;能力的「選擇」「授權」「憑證」是三條不同的軸,不可混談。

## At a Glance

```
                 agents-shared-capabilities/         (純集體 catalog,零 per-agent 檔)
                   skills/<name>/SKILL.md …
                   mcp/registry.yaml                 (server 定義;secret 只放參照)
                   templates/  tools/{sync.sh,lint.js,projectors/}
                         │
   agent-bot/{uid}/  ────┤  (cold 命名空間:該 agent 專屬能力 skills/ mcp/)
   ~/personal/       ────┤  capabilities.md (enable,預設全關) + permissions.md (授權)
                         │
                    tools/sync.sh   ← 讀 catalog + agent 命名空間 + capabilities.md,吃 enable 過濾
                         │  投影
        ┌────────────────┼────────────────┬──────────────┐
   ~/.claude/skills/  ~/.codex/skills/   antigravity      opencode
   .claude.json(MCP) config.toml(MCP)    …               …
        └─────────────── 執行時一律過 Hot Permission Boundary(permissions.md)
```

兩道閘:**閘 1 存在**(enable→sync 決定接不接進來)、**閘 2 授權**(permissions 決定准不准跑)。憑證走參照,值留各 host 本地。

## Approaches Considered

### 外部先例對照(授權 × 能力管理)

| 工具 | 授權模型 | MCP | Skill | 供應鏈信任 |
| --- | --- | --- | --- | --- |
| **Antigravity**(Google) | fine-grained `action(target)`,三張清單,**Deny > Ask > Allow**;用 `AGENTS.md` | global config + per-project 只放行需要的 tool | 原生 Anthropic Agent Skills(progressive disclosure);subagent 由主 agent 決定給哪些工具 | per-project 收斂縮小暴露面 |
| **opencode** | 每個 tool key 設 `ask`;**MCP tool 與內建工具同一權限模型** | stdio/SSE 註冊;`<server>_*` glob 全關再逐 agent 開;`mcp auth` OAuth,token 存本地 `mcp-auth.json` | per-agent 工具過濾 | read-only token / 窄 OAuth scope |
| **OpenClaw** | 多層 cascade(global/provider/agent/session/sandbox);**skill allowlist 只是可見/載入過濾,非 shell 授權邊界** | 走 tool | `SKILL.md`;global `skills/` vs agent-local `workspace/skills/` | ⚠️ marketplace「裝了就跑、全權限、無審核/簽章/能力宣告」——多篇 arXiv 點名為供應鏈風險 |
| **Hermes**(Nous) | command approval 白名單(較陽春) | — | **任務後自動蒸餾 skill** 到 `~/.hermes/skills/`(相容 agentskills.io) | ⚠️ skill 無審核自我修改 |
| **OpenAB**(gateway) | 不管授權,交各 CLI | 不管,交各 CLI | 不管,交各 CLI | 提供 config cron + lifecycle hook,可跑週期維運 |

**讀法**:授權面業界收斂到「**統一權限模型 + Deny>Ask>Allow**」(Antigravity、opencode、OpenClaw)——正是我們 `permissions.md` + Hot Permission Boundary 已有的形狀,連 precedence 都獨立撞同一組。可借:Antigravity/opencode 的「global 定義 → per-scope 收斂」(⇒ enable);OpenClaw 的「載入過濾 ≠ 授權邊界」血淚教訓(⇒ enable/permissions 分軸);opencode 的「OAuth token 本地、與 config 分離」(⇒ secret 參照);Hermes 的「任務後蒸餾迴圈」但保人工審核。要避:OpenClaw 的 install-and-run 供應鏈風險(⇒ vendored + PR + ask)。

### 設計岔路

- **repo 顆粒度**:單一 `agents-shared-capabilities`(skills+MCP 合一)vs 兩個獨立 repo。→ 選合一:兩者共用同一套 sync/lint/permission 機器。
- **shared/personal 切法**:repo 內設 `personal/` 子夾 vs 扁平 catalog + 個人能力落 agent cold 命名空間。→ 選後者(見 Decisions 3)。
- **enable 落點**:共享 repo 的 `enable/{uid}.yaml` vs `~/personal/capabilities.md`。→ 選後者(見 Decisions 4)。

## Decisions

1. **能力當 Warm 層 artifact 治理,不另立體系。** 採開放 Agent Skills 格式(`SKILL.md`,frontmatter `name`+`description`)+ host-agnostic MCP registry;兩者都是「host-agnostic 意圖 → per-runtime 投影」,與 `permissions.md`、`AGENTS.md` 同構。
2. **單一正本 catalog `agents-shared-capabilities`,經 projector 投影。** skill 逐一 **symlink** 進各 runtime skills 目錄;MCP registry 投影成各 runtime 設定(Claude Code `.claude.json`、Codex `config.toml [mcp_servers]`…)。每新增一個 runtime = 加一支 projector。
3. **共享 repo = 純集體 catalog,扁平、無 personal 子夾。** 個人專屬能力放該 agent 的 cold 命名空間 `agent-bot/{uid}/{skills,mcp}/`(本地掛載),對齊 `~/personal` 指進 `agent-bot/{uid}/personal` 的既有手法。撞名 precedence:**個人覆蓋 catalog**。好處:共享 repo 零 per-agent 檔,子樹可乾淨抽離(合 `shared ↛ agent-bot` 紀律)。
4. **選擇與授權為正交兩軸,分兩檔。** `~/personal/capabilities.md`(enable,per-agent,cold 版控,**預設全關**,cite `shared/capabilities-schema.md`)決定「裝/載入哪些」;`permissions.md` 決定「准不准跑」。`sync` 同時讀兩者。理由:載入/可見過濾 ≠ shell 授權邊界(OpenClaw 教訓);context 收斂與授權無關;塞進 permissions 會污染其 host-agnostic 授權契約並失去三態/precedence 語意。
5. **授權整合沿用既有 pattern。** capabilities repo:push/PR = allow、merge = ask。引入外部 skill / 啟用外部 MCP server = **ask**(供應鏈閘)。`SKILL.md` 的 `allowed-tools` 只**宣告足跡**,不是授權;skill script 與 MCP tool 執行時一律過 Hot Permission Boundary,**不另立閘**。
6. **外部能力:先審 + 釘版本 + vendored。** PR merge = review 事件;`SKILL.md`/registry 帶 capability 宣告 + `pinned-ref` + `checksum`,由 `lint.js` 驗;**不 live-link 遠端 marketplace**。對映學界對 install-and-run 供應鏈風險的建議。
7. **憑證與可攜設定分離。** registry 只存 secret **參照**(`env:` / `op://` / `vault:`…),真值由**可抽換 resolver** 於 sync/runtime 取。MVP resolver = env(本地 `.env`,gitignore);升級成 keychain/1Password/Vault/SOPS 只換 resolver、**不動 registry**。MCP OAuth token 交各 runtime 本地存,不入正本。
8. **供給流程收斂為冪等 `bootstrap.sh`。** clone 三個正本 → 從 template 生 `~/personal` → `sync` → reload runtime。維運用 OpenAB config cron 跑週期 `sync` 漂移檢查 + lint。

## Lessons learned(可重用,踩過的雷)

（決策階段尚未實作;PoC / 首次落地後回填:symlink vs 各 runtime skill 探索行為差異、MCP 投影格式差異、撞名/precedence 實際邊界、secret resolver 抽換成本。）

## Rejected alternatives

- **把 skill/MCP 寫進各 runtime 內建 memory store**(`~/.codex/memories` 等)——違反「memory 只走 agents-memory」;那些 store 只留薄指標。
- **enable 併入 `permissions.md`**——用安全檔兼差做配置,污染 host-agnostic 授權契約、失 context 收斂語意(見 Decisions 4)。
- **共享 repo 內設 `personal/` 子夾**——製造 per-agent 跨耦合,子樹無法乾淨抽離。
- **skills、MCP 拆兩個 repo**——共用同一套 sync/lint/permission 機器,合一更簡。
- **live-link 外部 skill/MCP marketplace**——供應鏈風險(OpenClaw 前例);改 vendored + PR + ask。

## Consequences

- 一次 `git pull` + `sync` 同步所有 coding agent;新 agent 走 `bootstrap.sh` 一鍵接入;每個 agent 差異只剩 `personal/`、`capabilities.md`、本地 secrets。
- 需為每個 runtime 維護一支 projector;runtime 各自的 reload 語意不同(Codex 需重啟)。
- 互動式 OAuth 無法可攜,仍需各 host 人工完成;正本只承載定義。
- 待辦(採行時另開 handoff,stage 規劃/實作中):`agents-shared-capabilities` 骨架、`shared/capabilities-schema.md`(warm)、`sync.sh`/`lint.js`/`projectors/`、secret resolver、`bootstrap.sh`、`permissions.md` 新規則三條。
- 「任務後蒸餾出可重用 skill」可掛上交付流程收尾步驟(見「Hermes 機制對照」分析),但保人工審核(PR merge 才生效)。

## Addendum:MCP facade-default routing(2026-09-05)

實作 octobroker 時新增的決策,補強原 Decisions 5/7(當時把帶 secret 的 MCP server 當 agent-facing、header 解析進 runtime config)。

**決策**:MCP server 預設**藏在 OAB MCP Facade 後**。registry 每個 server 標 `route`:
- **`facade`(預設)**:註冊為 facade 背後的 source(寫 openab `~/.openab/agent/mcp.json`);agent runtime **只連 loopback facade(127.0.0.1:8848)、不持任何 upstream key**;secret 以 `${env:VAR}` 由 openab facade 解析。
- **`direct`**:直接投影進 runtime MCP config。例外用途:① facade 本身(`oab-facade`)② 沒有 openab facade 的 host ③ facade 不能代理的 source。

**理由**:
1. **信任邊界**:agent(不可信程式碼)永不持 upstream 憑證;key 留 facade 側、不落 persistent HOME。取代早前「把 key 注入 runtime / 寫進 `~/.claude.json`」的方案。
2. **跨 Coding Agent 一致**:每個 runtime 只註冊同一個 loopback facade;新增 MCP 在 facade 後設定一次、全 runtime 共用,消除各家 header/auth 格式差異(codex `env_http_headers` vs claude `headers`…)。
3. **集中最小權限**:facade `tool_filter` + upstream 自身 per-agent 政策(如 octobroker default-deny 唯讀白名單 + repo scope)雙層縱深防禦。

**部署(整合進本機制)**:`sync.js --render` 於 pod 外從 registry + 該 agent `capabilities.md` 產出設定產物 → commit 進 agents-infra overlay → configMap 掛 `/etc/openab/mcp` → openab `pre_boot` `mcp-apply` merge 進 HOME(option C;GitOps、可審核衍生物,pod 不 clone、不 on-pod sync)。

**Lessons learned(實作回填)**:
- openab MCP source `headers`(含 `${env:}`)是 **#1511(2026-08-27)** 才有;`0.10.0-beta.3` **沒有** → facade 送不出 upstream header(octobroker 回 `missing X-Octobroker-Key`)。需 pin 含此功能的 image(先 nightly by digest,待正式版切回)。
- openab `[hooks.pre_boot].on_failure` 只吃 `abort|warn`(非 `continue`);無效值 → config parse fail、容器 CrashLoopBackOff。
- 大 image(~570MB)首拉可能超過 CD `rollout status` 逾時而**誤報**失敗;pod 隨後 ready。
- 首個 facade source = octobroker(GitHub 唯讀 MCP),於 claudecode1 **驗證通過(2026-09-05)**。

## Addendum:pod 的 skill 投影(2026-09-08)

部署 cfdrop 到 claudecode1 時暴露 Decision 2 的一個**盲點**:「skill 逐一 symlink 進各 runtime skills 目錄」是 **dev-only** —— symlink 的來源是**本地 catalog checkout**(`sync.js` 的 `skillSource` → `REPO/skills/<name>`)。但 **pod 沒有 catalog checkout**(option-C 刻意不 clone),所以 enabled catalog skill 從沒進過 pod 的 `~/.claude/skills`,claudecode1 只列得出內建 skill、cfdrop 不在。MCP 早有 option-C(render→configMap→pre_boot)填這條,skill 卻沒有 —— 因為在此之前沒有 catalog skill 被 enable 到 pod。

**決策**:skill 走與 MCP/bin **同一條 option-C**。`sync.js --render` 多吐一份 **`skills-manifest.json`**(該 agent enabled skills 的檔案 SKILL.md + assets/scripts…,base64、排序 → deterministic 可 `--check`)→ configMap 掛 `/etc/openab/mcp` → pre_boot **`skills-apply.js`** 把檔寫進 `~/.claude/skills/<name>`(path-escape 防護;`.catalog-managed.json` marker 供 GC 被 disable 的 skill)。dev 仍用 symlink(有本地 checkout);pod 用 render+write —— 兩者殊途同「投影」。

**理由**:① 對齊既有 pod capability 管線(mcp-apply/bin-apply),不新增 pod clone;② **per-agent**(由各 agent `capabilities.md` 驅動);③ HOME 在持久 PVC → 寫一次跨重啟保留。

**邊界**:configMap 上限 ~1 MiB → 很大的 skill(大 assets/scripts)需改用較重的傳輸(pod 端 sparse-checkout 或 init pull);render 逼近上限會 warn。

**落地**:catalog `sync.js --render` 加吐 skills-manifest(agents-shared-capabilities #17);agents-infra claudecode1 加 `skills-apply.js` + manifest + pre_boot 一行(agents-infra #41)。**實機驗證通過(claudecode1,2026-09-08)**:Agent Skills 列出 cfdrop。

## Addendum:pod projection 改 no-node + 第二 runtime(antigravity1)(2026-09-09)

上線第二個 agent 時兩件事驅動重構:

1. **no-node、runtime-portable**:apply glue 原本是 node 腳本;**antigravity image 沒有 node**(curl/tar/unzip/sha256sum/base64 有)。改成 **POSIX sh** + sh-friendly render 產物:`bin-manifest.json` → **`bin-install.tsv`**(每 tool×platform 一列),`skills-manifest.json` → **`skills.tar.b64`**(deterministic tar + base64)+ `skills.list`。skills-apply/bin-apply 改 `.sh`(canonical 在 catalog `templates/pod-apply/`)。`mcp-apply` 暫留 node(只在 claudecode1、node 在;antigravity 尚無 MCP)。PR:catalog #19、agents-infra #45。

2. **skill 目錄依 runtime 參數化**:`skills-apply.sh` 用 `SKILLS_DIR` env;claudecode1 = `~/.claude/skills`、antigravity1 = **`~/.gemini/antigravity-cli/skills`**。

3. **PATH 依 image 不同**:claudecode1(node image)`/home/node/bin` 本就在 PATH,設 `BIN_INSTALL_DIR=$HOME/bin` 即可;**antigravity1(debian image)PATH 上一個可寫目錄都沒有** → 必須用 container `env.PATH` 明確前綴 `/home/agent/bin`(保留原容器 PATH)。

**Lessons learned**:
- **`sh -lc`(login shell)會被 /etc/profile 重設 PATH**,測 `command -v` 出現 127 是假象;要用 **`sh -c`(non-login)** 驗 agent 實際 PATH。antigravity1 non-login PATH 確認為 `/home/agent/bin:/usr/local/sbin:...`、cfdrop resolve → **實機驗證通過(2026-09-09,兩 agent 皆列出並可執行 cfdrop)**。
- 殘留風險:若某 runtime 用 **login shell** exec 工具,container `env.PATH` 會失效,需改在 login PATH(`~/.profile` / profile.d)加 bin 目錄。目前兩 runtime 走 non-login,未觸發。
- 一個 skill/binary(cfdrop)現跨 **兩種 runtime**(Claude Code / Antigravity)驗證,佐證 catalog 的 runtime-agnostic 設計。

## 相關 ADR

- [ADR 0005](0005-hook-capability-provisioning.md) — hook 納入供給(第三軸)
- [ADR 0006](0006-binary-dependency-provisioning.md) — binary/CLI 依賴供給(第四軸)
- [ADR 0007](0007-authorization-projection.md) — 執行授權投影
- [ADR 0008](0008-tooling-rust-unification.md) — 工具統一為單一 Rust binary
