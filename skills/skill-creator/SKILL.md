---
name: skill-creator
description: Create a new Agent Skill or revise an existing one (SKILL.md frontmatter, instructions, scripts/, references/, assets/) following the Agent Skills specification and this catalog's conventions, then validate and optionally test it. Use when the user wants to make, write, scaffold, update, refactor, validate, or test a skill, improve a skill's description or triggering, or turn a workflow from the conversation into a reusable skill — e.g. 建立 skill、寫一個 skill、把這個流程做成 skill、改 skill、skill 沒被觸發. Not for memory notes, ADRs, MCP servers, or hooks.
metadata:
  short-description: Create, update, and validate Agent Skills
capability:
  tools: [Read, Write, Edit, Bash]   # authors skill files + runs scripts/validate.sh (declaration, not authorization)
  network: false
source: built-in   # authored for this catalog; synthesized from anthropics/skills@b0cbd3d skill-creator, openai/codex@5e32f72 samples/skill-creator, agentskills/agentskills@217be54 specification
pinned-ref: n/a
checksum: n/a
---

# Skill Creator

建立或修改 skill,讓之後的 agent 在特定任務上做出更好的決策——而不是約束不相關的工作。適用任何支援 Agent Skills 的 runtime(Claude Code、Codex、Antigravity…)。

## 原則

- **假設執行者已經很能幹。** 只寫會改變它決策、或提升產出的資訊。刪掉通用建議、重複指示、臆測的邊界情況,以及沒有實質釐清作用的範例。
- **保留使用者的意圖與範圍。** skill 是支援被要求的任務,不是替使用者換工具、擴大任務、改無關設定,或暗示可以做額外的對外動作。不要把單一例子、過去一次失誤或個人偏好升格成通用規則。「准做這件事」不等於擴大範圍或執行權限;會重試或會對外改動的流程,要寫明與風險相稱的停止條件。
- **具體度與風險相稱。** 開放式工作描述目標與判斷準則,讓執行者自己選做法;有偏好形狀的工作給範例或可設定的腳本;只有偏離會造成具體問題時(正確性、安全、權限、脆弱流程)才寫死步驟、腳本或絕對語氣。
- **解釋 why,少用全大寫。** 想寫 ALWAYS / NEVER 時,先試著改成說明理由——懂原因的模型比背規則的模型更能處理沒預料到的情況。
- **探索要便宜又精準。** `name` + `description` 在 skill 載入前就會被看到,寫清楚「做什麼 + 何時用」;只有能防止常見誤觸發時才加排除條件。
- **漸進揭露。** 共用目的、關鍵限制、路由放 `SKILL.md`;只在特定情境需要的大段內容放 `references/`,並寫明何時讀。簡單、自足的 skill 不需要路由層或額外檔案。
- **不出乎意料。** skill 內容被描述出來時不應讓使用者意外;不寫惡意程式、不寫為了未授權存取或資料外洩而設計的 skill。

## Skill 結構

```text
skill-name/
├── SKILL.md        必要:YAML frontmatter(name + description)+ Markdown 指示
├── scripts/        選用:確定性、會重複的邏輯(執行即可,不必載入內容)
├── references/     選用:特定情境才讀的文件(schema、API、領域規則、細部流程)
└── assets/         選用:放進產出物的檔案(範本、圖示、字型、boilerplate)
```

只建任務真的需要的目錄與檔案。不要加 README、安裝說明、changelog 或重複的 quick reference。references 離 `SKILL.md` 一層、用相對路徑連結;超過 ~300 行的 reference 加目錄或可搜尋的關鍵詞。

## 流程

依請求調整——新的複雜 skill 可能每步都要走;小幅修改可能只要一個 focused edit + 驗證。

### 1. 釐清意圖

若對話中已經有想做成 skill 的流程,先從中萃取:用了哪些工具、步驟順序、使用者做過的修正、輸入/輸出格式。要確認的是:

1. 這個 skill 要讓 agent 做到什麼?
2. 什麼請求/情境該觸發它?
3. 產出長什麼樣?
4. 要不要測試?產出可客觀驗證的(檔案轉換、資料萃取、固定流程)適合;主觀的(文風、設計)通常不必。

只在缺的資訊確實重要、又無法合理推斷時才問;使用者已經講清楚就直接做。

### 2. 決定放哪裡

使用者指定位置就照做。在本 agent 體系內:

- **所有 agent 共用** → `agents-shared-capabilities` catalog 的 `skills/<name>/`,走 PR(merge = review 閘)。
- **單一 agent 專屬** → 該 agent 在 cold memory 的命名空間 `agent-bot/<uid>/skills/<name>/`。
- **不要直接寫進 runtime 的 skill 目錄**(如 `~/.claude/skills/`、`~/.codex/skills/`)——那是 `capsync` 的投影目標,下次 sync 會被覆蓋。只有在這套體系之外使用時才直接放 runtime 目錄。

先分清楚是不是 skill:可重複執行、帶步驟/腳本/範本的**程序** → skill;行為準則、事實、決策紀錄 → memory note 或 ADR,不是 skill。

### 3. 規劃可重用資源

從實際會遇到的請求反推,只在有具體好處時才建:

- 同樣的轉換每次都要重寫 → `scripts/`(優先 POSIX sh;需要其他直譯器或 native binary 時,在 catalog 用 `requires:` 宣告並登錄 `bin/registry.yaml`)。
- 每次都要重新摸索的 schema / API / 規則 → `references/<topic>.md`。
- 產出物要用的範本或素材 → `assets/`。
- 多個互斥模式(例如不同雲端供應商)→ 各一份 reference,`SKILL.md` 只放選擇準則,讓執行者只讀相關那份。

### 4. 寫 frontmatter

最小可用:

```yaml
---
name: my-skill
description: <做什麼>. Use when <何時用:使用者會說的話、情境、關鍵詞>.
---
```

在 catalog 內,照 `templates/SKILL.template.md` 補 `metadata.short-description`、`capability`、`source`、`pinned-ref`、`checksum`(與選用的 `requires`)。欄位限制、catalog 擴充鍵與規範的分歧、description 寫法 → 讀 [references/frontmatter.md](references/frontmatter.md)。

命名:小寫、數字、單一連字號,≤64 字元,資料夾名 = `name`,偏好短的動作導向名稱,有助探索時以工具或領域當前綴。

### 5. 寫 body

寫另一個 agent 執行任務所需的指示就好:期望的結果、不明顯的脈絡、真實的限制、相關的 reference 或工具。用祈使句;需要固定輸出格式時給範本,需要風格時給一兩個範例。不要在沒必要時規定固定結構、流程或步驟數。整份 `SKILL.md` 保持在 500 行內,接近時把細節拆進 `references/`。

範例、測試資料只用虛構資料;任何金鑰、密碼、token 一律寫成 `${API_KEY}`、`<YOUR_TOKEN>` 之類的 placeholder。

寫完用新的眼光再讀一遍:每一段都在改變執行者的決策嗎?不是就刪。

### 6. 驗證

```sh
scripts/validate.sh <path/to/skill>                # 規範 + catalog 慣例;catalog 擴充鍵只 warn
scripts/validate.sh --strict-spec <path/to/skill>  # 要通過 skills-ref validate 時
```

`scripts/` 路徑相對於本 skill 的目錄。它檢查 frontmatter、命名、長度上限、未完成的 placeholder、相對連結是否存在、腳本是否可執行。在 catalog repo 內另跑 `capsync lint --catalog .`(CI 也會跑)。新增或修改的腳本要實際跑過。

驗證通過只代表格式對,不代表 skill 會做出好的決策——還要確認 description 有區辨力、指示保留了使用者意圖、references 找得到。

### 7. 測試(視需要)

skill 夠複雜或風險夠高時,用獨立的執行者做行為測試(with / without skill 對照)與觸發測試 → 讀 [references/evaluation.md](references/evaluation.md)。一般的建立或小改不需要。

### 8. 交付

- catalog:開 PR,說明這個 skill 做什麼、何時觸發、驗證結果。
- **建立 ≠ 啟用 ≠ 授權**:skill 進 catalog 後,還要在各 agent 的 `capabilities.md` 啟用才會被投影;skill 要執行的指令另需 `permissions.md` 授權。這兩步屬於各 agent 的設定,不要在建立 skill 時順手改,除非使用者要求。

## 修改既有 skill

- 先讀整個 skill(含 scripts / references 的呼叫者)再動;刪除資源前確認沒有地方還在用。
- 保留 `name`、資料夾名與既有 frontmatter 欄位;只改要改的部分。
- 依實際使用或已觀察到的失敗做**窄修正**,不要因單一案例累積一堆通用規則。
- 外部來源(vendored)的 skill:最小 body 編輯,並在 provenance 註明與上游分岔的原因。
