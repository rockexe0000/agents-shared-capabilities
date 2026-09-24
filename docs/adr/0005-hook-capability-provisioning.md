---
title: "ADR 0005:Coding Agent hook 納入 capability 供給(第三軸,canonical 事件 + allow/deny 授權前移)"
---

> Vendored 設計快照:本 ADR 由團隊內部設計記錄複製而來,作為本 repo 的自足設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)已移除或改為本地 `docs/adr/` 連結。

# ADR 0005:Coding Agent hook 納入 capability 供給(第三軸,canonical 事件 + allow/deny 授權前移)

**Status**:Accepted · **決策日**:2026-09-07 · **延伸**:ADR 0002

## Context

owner z 希望 Coding Agent 的 **hook** 跟 skill / MCP 一樣,透過 `agents-shared-capabilities` 集中供給(單一 catalog → per-runtime 投影 + per-agent enable-list),而非各 host 各自在 `settings.json` 手維、跨 agent 漂移。ADR 0002 已為 skill/MCP 建好「host-agnostic 意圖 → per-runtime 投影 + 兩道閘」的骨架;本 ADR 把 hook 接成第三軸。

但研究(見下表)確認 hook 與 skill/MCP **不是乾淨的平行**,有兩個必須先解的差異:(1) hook 跨 host **沒有標準**——事件名與設定形態各家不同;(2) hook 由 harness 在事件上**自動執行 shell**,不經 agent 的 tool-call 路徑,因此**繞過 Permission Boundary(閘 2 對 hook 不 fire)**。

### 研究:openab-支援的 Coding Agent hook event 比較

以 canonical 事件對照各 host 原生事件(只列有 hook 機制者;其餘 ACP agent 無 hook 即 skip):

| Agent | pre-tool | post-tool | session-start | user-prompt-submit | stop | 設定落點 / 形態 |
|---|---|---|---|---|---|---|
| Claude Code | PreToolUse(可擋) | PostToolUse | SessionStart | UserPromptSubmit | Stop(可擋) | `~/.claude/settings.json` → `hooks`(JSON) |
| Codex | PreToolUse(可擋) | PostToolUse | SessionStart | UserPromptSubmit | Stop(可擋) | `~/.codex/hooks.json` 或 `config.toml [hooks]` |
| Gemini CLI | BeforeTool(可擋) | AfterTool | session events | ~ | ~ | `~/.gemini/settings.json` → `hooks`(JSON) |
| Antigravity | PreToolUse(可擋) | PostToolUse | ✗(PreInvocation) | ✗ | Stop(可擋) | `hooks.json`(`.agents/` 或 `~/.gemini/config/`) |
| opencode | tool.execute.before(可擋) | tool.execute.after | session.created | message events | session.idle | **JS plugin 事件匯流排(非 JSON)** |
| Cursor | preToolUse(可擋) | postToolUse | sessionStart | beforeSubmitPrompt | stop(可擋) | JSON hooks |

來源:各家官方 hook 文件(Claude Code / Codex / Gemini CLI / Antigravity / opencode / Cursor,2026-09 擷取)。

**導出**:完全交集只有 `pre-tool` / `post-tool`(六家全有、`pre-tool` 幾乎都可 block);`session-start` / `stop` 多數有;`user-prompt-submit` 過半有。故取極小字彙 = **pre-tool · post-tool · session-start · stop**,加選配 **user-prompt-submit**。

## At a Glance

```
   agents-shared-capabilities/
     skills/ …          mcp/registry.yaml          hooks/registry.yaml   ← 新增第三軸
                                                     (canonical 事件 + command;外部釘版本/checksum)
                              │
   ~/personal/capabilities.md   skills: / mcp: / hooks:(effect: allow|deny,預設 deny)  ← enable + 授權
                              │  sync 投影
        ┌─────────────────────┼──────────────────────┐
   claude-code projector   codex projector        …(逐一補;無 hook 機制者 skip)
   settings.json.hooks     hooks.json             opencode = JS plugin(形態不同)

   canonical 事件 → 各 projector map 到 host 原生;對應不到 → skip + log
   授權:hook auto-exec 繞過 Permission Boundary → 無 ask,只有 allow/deny,審查前移到 catalog PR
```

## Approaches Considered

### 方案 A(採用)— hook = 第三 capability 軸,canonical 事件 + per-projector + allow/deny 前移
- Pros:與 skill/MCP 同一機制、同一 enable-list、同一漂移檢查;跨 agent 單源不漂移;沿用 ADR 0002 的 projector/lint/供應鏈骨架。
- Cons:每 runtime 要一支 hook projector(形態各異);canonical 事件是抽象、有對應不到的 host。

### 方案 B — 只做 Claude Code、直接存原生 hook JSON、不抽象
- Pros:最省。
- Cons:成了唯一「不 host-agnostic」的軸,違反 ADR 0002 D1;換/加 runtime 就破。否決為唯一手段(但 MVP 先只實作 claude-code projector 是可接受的漸進)。

### 方案 C — hook 另立系統(如放 agents-infra)
- Pros:與 capability 解耦。
- Cons:provisioning 分裂成兩套、enable/授權/漂移各自為政。否決。

## Decisions

1. **hook = 第三 capability 軸,鏡像 MCP registry。** `hooks/registry.yaml`(host-agnostic 定義)+ `capabilities.md` `hooks:` enable-list + per-runtime projector + `lint.js` 驗。與 skill/MCP 同構(ADR 0002 D1/D2)。
2. **極小 canonical 事件字彙**:`pre-tool` · `post-tool` · `session-start` · `stop`(+選配 `user-prompt-submit`)。各 projector 把 canonical → host 原生事件;**對應不到的事件 / 無 hook 機制的 runtime → skip + log**(如 MCP direct route 遇未安裝 runtime)。字彙由上表交集導出,日後放寬只需加 canonical 名 + 各 projector 補 map。
3. **hook 授權 = allow/deny,無 ask(授權前移)。** hook 由 harness auto-exec、不經 agent tool-call,**Permission Boundary(閘 2)對 hook 不 fire**;`ask`(每次確認)在 fire 當下**無互動點**,語意不成立。故 hook 只有二態,授權**前移到 enable/catalog-review**:落 `capabilities.md` `hooks: effect: allow|deny`,**預設 deny/全關**,設 `allow` 是需 verified-owner 授權的決定(catalog PR = review 事件)。
4. **外部 hook 供應鏈(沿用 ADR 0002 D6)**:vendored + 釘版本 + checksum,不 live-link;因 auto-exec,catalog 收 hook 的審查標準**高於** skill。「把外部 hook vendored 進 catalog」仍是 ask 級供應鏈動作(repo 動作,PR-merge 為閘)。
5. **projector 形態各異,MVP 先 claude-code。** Claude/Gemini 寫 `settings.json.hooks`、Codex/Antigravity 寫獨立 `hooks.json`、**opencode 是 JS plugin 事件匯流排(得產 plugin 檔、非 merge JSON)**。先實作 claude-code projector,其餘逐一補(每加一 runtime = 加一支 projector,ADR 0002 D2)。claude-code projector 沿用 MCP projector 的 safe read-modify-write + `.bak` + 只動自己標記的 entry(冪等 add/remove,不清 user hook)。
6. **兩軸正交仍成立於「選擇 vs 執行」,但對 hook「選擇即授權」。** schema 明寫「hook 自動執行、繞過 runtime 授權閘」警語,提醒收 hook 的重量高於 skill/MCP。

## Lessons learned(可重用,踩過的雷)

- **hook 打破 skill/MCP 的 enable ⊥ authorize 正交**:因為它 auto-exec、繞過 runtime 授權閘,enable ≈ 授權。修正不是硬套三態,而是**砍掉 ask、把授權前移到 enable/review**——三態的 `ask` 需要 fire 當下有互動點,hook 沒有。
- **projector 冪等標記法隨 host config 結構而異,沒有單一寫法通吃**:Claude 的 `hooks` 以「事件」為 key、值是 group array → 用 **entry 內 `_managedBy` 標記**過濾自己投的;Antigravity 的 `hooks.json` 以「hook 名稱」為 top-level key → 用 **`asc:` key 前綴**辨識。每支 projector 依其 config 形態選最不會誤傷 user hook 的辨識法。
- **部分覆蓋要誠實 skip+log,不硬塞事件**:Antigravity 無 session-start / user-prompt-submit,canonical 對應不到就跳過並記錄,而非造一個假事件——寧可覆蓋面小而誠實。
- **動 shared schema body 前先查 watermark**:capabilities-schema 加 `hooks` 區塊本會 stale 指它的 `cite_synced_hash`,但實測 shared scope 內無人以 hash 指它(capabilities repo 的 template 只有 `cite_ref`、無 hash)→ 無 re-stamp cascade。與 item 1/2 同一紀律:碰被浮水印的 schema body 前先算 cascade。
- **本地 catalog checkout 可能落後 main**:實作前 local 落後(main 已有 secret-backends resolver / opencode projector),差點基於舊碼改;動工前先 `fetch` + rebase,別假設 local 是最新。
- (opencode 的 JS plugin 形態尚未實作,待接手時回填。)

## Rejected alternatives

- Claude-only 存原生 JSON、不抽象(唯一非 host-agnostic 軸,違 0002 D1)。
- hook 另立系統 / 放 infra(provisioning 分裂)。
- hook 沿用 permissions.md 三態 `ask`(auto-exec 在 fire 當下無從問,語意不成立)。

## Consequences

- capability 供給多一軸;每 runtime 要一支 hook projector(先 claude-code)。
- `capabilities-schema.md`(warm)加 `hooks` 區塊——會 stale 其 cite watermark(目前僅 `capabilities.template.md`,無 agent 有 capabilities.md),re-stamp 範圍小、屬預期。
- `permissions.md` 的 hook 授權**不新增三態規則**(授權落 capabilities.md allow/deny);唯「vendored 外部 hook 進 catalog」沿用既有供應鏈 ask 精神。
- 後續:補 codex / gemini / antigravity / opencode projector;字彙按需擴充。

## 相關 ADR

- [ADR 0002](0002-agent-capability-provisioning.md) — capability 供給機制(projector/lint/供應鏈骨架);本 ADR 為其延伸的第三軸。
