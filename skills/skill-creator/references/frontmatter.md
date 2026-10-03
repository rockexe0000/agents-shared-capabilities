# Frontmatter 速查:Agent Skills 規範 + 本 catalog 擴充

寫或改 frontmatter 時讀。規範原文:<https://agentskills.io/specification>。

## 規範欄位(所有相容 runtime 共通)

| 欄位 | 必填 | 限制 |
|------|------|------|
| `name` | 是 | 1–64 字元;僅 `a-z` `0-9` `-`;不可以 `-` 開頭/結尾、不可有 `--`;**必須等於資料夾名** |
| `description` | 是 | 1–1024 字元;說明「做什麼 + 何時用」 |
| `license` | 否 | 授權名稱,或指向隨附授權檔(如 `Proprietary. LICENSE.txt has complete terms`) |
| `compatibility` | 否 | ≤500 字元;只有真的有環境需求(特定 runtime、系統套件、需連網)才寫 |
| `metadata` | 否 | string → string 的 map,放規範未定義的屬性;key 取得夠獨特以免撞名 |
| `allowed-tools` | 否 | 實驗性;空白分隔的預先核准工具清單(如 `Bash(git:*) Read`),各 runtime 支援度不一 |

額外慣例(非規範、但多數 runtime 會擋):`name` 不含保留字 `claude` / `anthropic`(本 catalog 的 `capsync lint` 也擋)。

## 本 catalog 擴充鍵(`agents-shared-capabilities`)

catalog 內的 skill 依 `templates/SKILL.template.md` 另帶:

```yaml
metadata:
  short-description: 一句話摘要(UI 顯示用)
capability:
  tools: [Bash, Read]   # 宣告足跡,不是授權;授權在各 agent 的 permissions.md
  network: false        # 是否連網
requires:               # 選用:要 shell out 的 native binary → bin/registry.yaml
  - name: some-cli
    min: "1.2.0"
source: built-in        # 或 external:<repo/url>
pinned-ref: n/a         # 外部來源填 commit/tag
checksum: n/a           # 外部來源填上游檔案 sha256
```

> **與規範的已知分歧**:`capability` / `requires` / `source` / `pinned-ref` / `checksum` 位於頂層,
> 規範的 reference validator(`skills-ref validate`)會以 "Unexpected fields" 拒收。Claude Code、
> Codex 等 runtime 實務上忽略未知鍵,且 `capsync` 依賴這些鍵,所以 catalog 內**照 catalog 範本寫**;
> 要發佈到 catalog 以外、必須通過 `skills-ref` 的 skill,改把這些資訊放進 `metadata`(string 值)
> 或拿掉,並用 `scripts/validate.sh --strict-spec` 檢查。

## 寫 description

description 是 runtime 決定「要不要載入這個 skill」的唯一依據(body 在觸發後才載入),所以:

- 同時寫**做什麼**與**何時用**;「何時用」的資訊放這裡,不要只寫在 body。
- 放使用者實際會講的關鍵詞與說法(中英文都會出現就都放)。
- 第三人稱、具體;避免「Helps with X」這種一句帶過。
- 只在真的會誤觸發時才寫排除條件(「Not for …」),不要列一長串能力清單或 catch-all。
- 模型傾向**少觸發**:措辭可以稍微積極(列出使用者沒點名、但其實需要它的情境),但不要積極到吸走無關請求。

好:

```yaml
description: Extracts text and tables from PDF files, fills PDF forms, and merges PDFs. Use when working with PDF documents or when the user mentions PDFs, forms, or document extraction.
```

差:

```yaml
description: Helps with PDFs.
```

## 更新既有 skill 時

- `name` 與資料夾名**不改**(改了等於另一個 skill,既有 enable 設定會失效)。
- 保留既有的 `metadata`、catalog 擴充鍵與其他 runtime 專用欄位;只動要改的部分。
- vendored(`source: external:…`)的 skill:做最小 body 編輯,並在 `checksum` 行的註解或 provenance 說明與上游分岔的原因。
