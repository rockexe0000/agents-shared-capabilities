---
title: "ADR 0007:執行授權投影(permissions.md → 各 runtime enforcement config)"
---

> Vendored 設計快照:本 ADR 由團隊內部設計記錄複製而來,作為本 repo 的自足設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)已移除或改為本地 `docs/adr/` 連結。

# ADR 0007:執行授權投影(permissions.md → 各 runtime enforcement config)

**Status**:Accepted · **決策日**:2026-09-10 · **落地**:ADR 0002 Decision 5 · **關聯**:ADR 0006

## Context

cfdrop 供給鏈跑通後(vendored → enable → skill 投影 → binary 裝 → PATH),**antigravity1 卻無法執行 cfdrop**:`agy failed: Error: permission check failed for command "cfdrop --version": user denied permission to run command`。claudecode1(Claude Code)可以跑。

診斷:這是 **閘 2(執行授權)**,不是供給/投影 —— cfdrop 已裝好且在 PATH。差別在 runtime 的**原生指令權限閘**:
- **agy(Antigravity CLI,debian 這台的 backend)** 讀 `~/.gemini/antigravity-cli/settings.json` 的 permission allowlist,且 **headless `-p` 模式會 honor 該 settings**(antigravity-cli CHANGELOG 明載);未 allowlist 的指令在無互動核准者時預設拒。
- **Claude Code** 那條路徑的授權已通(openab 的 ACP `session/request_permission` auto-reply 對 Claude 端有效)。

現況缺口:ADR 0002 Decision 5 說「`permissions.md` → 各 CLI enforcement config」的投影**只是概念、沒建**。enable(閘 1)+ 供給都做了,**執行授權(閘 2)沒有投影到 runtime**,所以較嚴格的 runtime(agy)擋下未授權指令。

判準:
- **enable ≠ authorize** 要守住;runtime 擋下未授權指令是**正確行為**,不是 bug。
- 授權是**安全決定**,預設要保守(secure by default),對齊系統既有「capabilities 全關 / permissions 未列→ask / 供應鏈→ask」。
- 以**最精簡的 debian image(agy)為 baseline**設計(較受限的環境),claude 對映;不烤入單一 runtime 假設。

## At a Glance

```
三軸回顧:
  vendored 進 catalog (組織供給,供應鏈 ask)
  → capabilities.md enable (選擇軸;sync 投影 skill/mcp + 裝 bin 到 host)
  → permissions/授權 (閘 2:runtime 准不准跑)   ← 本 ADR 補這條的投影

授權投影(在 enable 的投影管線裡多一支,產出閘 2 的 config):
  permissions.md(授權來源 + 旗標) ─┐
  enabled skills 的 requires(建議)─┤ sync --render → perms fragment
                                    ▼
   configMap → pre_boot perms-apply.sh → MERGE 進 runtime settings.json
      agy:   ~/.gemini/antigravity-cli/settings.json  permissions.allow: ["command(cfdrop)"]
      claude:~/.claude/settings.json                  permissions.allow: ["Bash(cfdrop:*)"]
      (deny 永遠覆蓋:deny > ask > allow)
```

## Approaches Considered

### 授權來源:A(自動)vs B(明確)vs blanket

- **A 自動授權**:enabled skill 的 `requires` binary → 自動寫進 runtime allowlist。Pros:enable 即可用、零摩擦。Cons:授權決定實質在 enable 當下下,弱化 enable≠authorize。
- **B 明確授權(採為預設)**:allowlist 只由 `permissions.md` 明確 allow 的產生;`requires` 只當建議清單。Pros:secure by default、enable≠authorize 守乾淨、每指令 owner 點頭。Cons:每上新 skill 多一次手動 allow。
- **blanket `--yolo` / trust-all**:agy 全放行。**否決**:違背最小權限,一顆壞 skill 就能亂跑。

**決定**:**預設 B**,但提供 **A 為可切換模式**(見 Decisions 3)——安全預設 + 需要時降摩擦。

### 開關放哪:permissions.md vs config.toml

- **permissions.md(採用)**:授權政策留在授權軸同一檔;天生吃 deny>ask>allow;`sync --render`(pod 外)當場能算出最終 allowlist。
- **config.toml(openab runtime,否決為預設位置)**:per-pod 好切,但把授權政策拆進 runtime plumbing,且 render 看不到 → 只能吐 superset 讓 pod 端過濾,較繞。

### 外部先例(runtime 權限格式)

| runtime | 權限 config | allow 條目 | headless 是否 honor |
| --- | --- | --- | --- |
| agy(Antigravity/Gemini 系) | `~/.gemini/antigravity-cli/settings.json` | `permissions.allow: ["command(cfdrop)"]` | 是(`-p` honor settings.json permissions,CHANGELOG) |
| Claude Code | `~/.claude/settings.json` | `permissions.allow: ["Bash(cfdrop:*)"]` | 是 |

## Decisions

1. **落地 ADR 0002 Decision 5:第四種投影 = 執行授權。** enable 的投影管線多一支 `perms-apply`,把「授權」投影進各 runtime 的**原生權限 config**(agy `settings.json` permission allow;claude `settings.json` permissions.allow)。**不用 blanket yolo/trust-all**——scoped 到具體指令。
2. **預設 B(明確授權)。** runtime allowlist = `permissions.md` 明確 allow 的集合;skill 的 `requires` **只當建議清單**(render 會把它列出來,方便 copy-paste 進 permissions.md)。
3. **A 為 permissions.md 旗標可切換。** permissions.md 加 per-agent 旗標 `authorize_skill_requires: auto | explicit`(**預設 `explicit` = B**)。設 `auto` = A:enabled skills 的 `requires` 自動推導成 allow 條目。**顆粒度**先做全 agent 一個旗標;未來要更細再做 per-skill。
4. **`permissions.md` deny 永遠覆蓋。** 不管 A/B,precedence **deny > ask > allow** 對自動推導出的 allow 一樣適用——owner 對某指令下 deny,蓋過一切。
5. **per-runtime 對映,baseline debian/agy。** 一支抽象「allow 指令 X」→ 各 runtime projector 映成原生格式(agy `command(X)`、claude `Bash(X:*)`)。新 runtime = 加一個對映。確切 settings.json schema 於實作時對 pod 上實檔/官方 pin 死(避免寫壞 —— agy 拒絕 unparseable settings)。
6. **走 no-node option-C,MERGE 不覆蓋。** `sync --render` 吐授權 fragment → configMap → pre_boot `perms-apply.sh`(POSIX sh)**merge** 進 runtime settings.json(保留 user/agent 既有設定;agy CHANGELOG 提到誤覆蓋 settings 會清掉別的設定)。與 skills/bin/mcp apply 同一條 pre_boot。
7. **`requires` 當建議清單的 UX。** 即使預設 B,render 把 enabled skills 的 `requires` 印成「建議 allow」條目,手動授權只需 copy-paste,不必自己查指令名。

## Lessons learned(可重用,踩過的雷)

- **claude 能跑、agy 不能** 的差別:openab ACP auto-reply(`crates/openab-core/src/acp/connection.rs`,正確 outcome wrapper)對 Claude 端夠用;agy 有自己的 settings.json 指令閘,headless `-p` 嚴格 honor,未 allowlist 即拒。授權必須投影到 agy 那份 settings。
- **不要 blanket `--yolo`**:能解一時但打開整台 shell,違反這套一路的最小權限。
- **agy schema 已釘死驗證(2026-09-11,antigravity1 實機)**:key 是 **`permissions`(複數)**,不是 `permission` —— 正本 `~/.gemini/antigravity-cli/settings.json` → `permissions: { allow: ["command(cfdrop)"], deny: [...] }`,條目 `action(target)` 格式(`command(cfdrop)` = 允許 cfdrop 指令任意參數)。**scoped `permissions.allow` 在 agy 1.1.13 headless `-p` 有效**(上游 headless bug #548/#565 未咬到 1.1.13;先前失敗純粹是我 key 少了 s)。所以 agy 不需退到 blanket。
- **fallback(僅備用)**:若某 agy 版本/runtime 的 headless 真的吃不到 scoped,退 `toolPermission: "always-proceed"`(blanket,標暫時性、追上游)。目前用不到。
- 實作待回填:merge 演算法(POSIX sh 無 jq 時如何安全 merge JSON,**不可覆蓋 `model`/`trustedWorkspaces`**);claude 端 `Bash(x:*)` glob 精確語意;agy tokenize 雷(CHANGELOG:`command(time)` 之類 tokenize 成零字會誤 match 全部)。

## Rejected alternatives

- **blanket `--yolo` / trust-all-tools**——太廣,違最小權限。
- **開關放 config.toml**——拆散授權軸 + render 要吐 superset;改放 permissions.md。
- **把授權併進 capabilities.md**——破壞「選擇 ≠ 授權」分離(ADR 0002 Decision 4);授權留 permissions.md。
- **預設 A(自動)**——摩擦小但弱化 enable≠authorize;改預設 B、A 可切換。

## Consequences

- 執行授權從此**系統化投影**:上新 skill 的指令依政策(B 明確 / A 自動)授權進 runtime;antigravity1 的 cfdrop 是首個對象。
- 新增維護面:per-runtime 權限 projector、`perms-apply.sh`(no-node,JSON merge)、permissions.md 旗標解析、每 runtime settings.json schema 釘死。
- **MERGE 不覆蓋** 是硬約束(別清掉 user/agent 既有 settings)。
- 待辦(見對應 handoff,stage 規劃/實作中):permissions.md 旗標 + schema、`sync --render` 授權 fragment、per-runtime 對映(agy/claude)、`perms-apply.sh`、antigravity1/claudecode1 overlay 接線、antigravity1 cfdrop 首驗。先手動 scoped-unblock antigravity1(需先確認 agy settings.json 實 schema)。

## 相關 ADR

- [ADR 0002](0002-agent-capability-provisioning.md) — 能力供給;Decision 5 = 本 ADR 要建的授權投影。
- [ADR 0006](0006-binary-dependency-provisioning.md) — binary 供給;`requires` 即本 ADR 的授權建議來源。
