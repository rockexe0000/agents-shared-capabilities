# 評測與迭代

skill 夠複雜或風險夠高、值得用實際行為驗證時讀。小改動或單純的 skill 不需要整套流程——`scripts/validate.sh` + 自己讀一遍通常就夠。

## 目錄

1. 行為測試(with / without skill 對照)
2. 評分
3. 觸發測試(description 準不準)
4. 依結果改 skill
5. 各 runtime 的差異

## 1. 行為測試

1. 寫 2–3 個**真實使用者會說的** test prompt(帶具體細節:檔名、欄位、情境),先給使用者看過。
2. 在**隔離的暫存工作區**跑,別讓產物進 repo:用 `~/tmp/<skill-name>-eval/iteration-<N>/<eval-name>/`(用 `~/tmp`,不要裸 `/tmp`——有寫檔沙箱的 runtime 只准寫 workspace 內)。
3. 每個 prompt 跑兩組,能並行就同時發出:
   - **with_skill**:告訴執行者 skill 路徑 + prompt + 輸出位置。
   - **baseline**:新 skill → 不給 skill;改既有 skill → 先 `cp -r` 舊版快照,指向快照。
4. 給執行者的只有:真實的請求、skill、必要的原始素材。**不要**透露預期答案、你懷疑的 bug 或你想要的結論——否則測到的是你的提示,不是 skill。

```text
Use the skill at <path/to/skill> to complete this request:
<realistic user prompt>
Save outputs to ~/tmp/<skill-name>-eval/iteration-1/<eval-name>/with_skill/
```

測試資料一律虛構;不要拿真實客戶資料、金鑰或內部網址當 test fixture。

## 2. 評分

- 能客觀驗證的(檔案產出、格式轉換、固定流程)→ 寫 assertion,名稱要一眼看懂在檢查什麼;能用 script 檢查的就寫 script,下輪可重用。
- 主觀的(文風、設計)→ 交給人看,不要硬湊 assertion。
- 測「可觀察行為或有意義的不變量」,不要測生成的措辭、標題或 regex 長相。
- 看完結果再看**過程**(transcript / 執行紀錄):執行者是否在 skill 引導下浪費步驟、是否每次都自己重寫同一段 helper(→ 該放進 `scripts/`)。
- 留意 with/without 都會通過的 assertion——它沒有區辨力。

## 3. 觸發測試

description 決定 skill 會不會被載入,值得單獨測:

1. 寫 ~20 個查詢,約一半 should-trigger、一半 should-not-trigger。
2. should-trigger:同一意圖的不同說法(正式/口語/有錯字/沒點名但明顯需要)、與其他 skill 競爭但應勝出的情境。
3. should-not-trigger:**近似但不該觸發**的案例(共用關鍵詞、相鄰領域)最有價值;明顯無關的查詢測不出東西。
4. 查詢要夠實質——簡單一步就能做完的請求,runtime 本來就不會去查 skill,拿來測沒意義。
5. 先給使用者審過查詢集,再跑;改 description 時保留一部分查詢不參與調整(held-out),避免對測試集過擬合。

## 4. 依結果改 skill

- **泛化,別過擬合**:skill 會被用在無數個 prompt 上,不是只有這幾個 test case。頑固的問題換個說法或換種做法,而不是加一條條針對特例的 MUST。
- **保持精簡**:刪掉沒發揮作用、或讓執行者多走冤枉路的段落。
- **解釋 why**:想寫全大寫 ALWAYS/NEVER 時,先試著改成說明理由。
- **重複工作 → 打包**:多個 test case 都自己寫了類似 helper,就把它做成 `scripts/` 並在 SKILL.md 指過去。
- 改完重跑全部 test case 進 `iteration-<N+1>/`(含 baseline);使用者滿意、回饋已空、或不再有實質進展就停。

## 5. 各 runtime 的差異

- **有 subagent**(Claude Code、Codex 等):照上面並行跑 with/without。
- **沒有 subagent**:自己讀 SKILL.md 依指示做,一次一個;這不夠獨立(你寫的、你也知道全部脈絡),跳過 baseline 與量化比較,改以使用者的質性回饋為主。
- 會花可觀時間/成本、需要額外授權、或會碰到正式環境的評測,先問過使用者。
