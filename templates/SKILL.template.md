---
name: <kebab-case,≤64,僅小寫/數字/連字號,不含 claude/anthropic>
description: <第三人稱;同時寫「做什麼 + 何時用」+ 關鍵詞;≤1024 字>
metadata:
  short-description: <一句話>
capability:
  tools: []          # 宣告足跡:要動的工具(非授權,授權在 permissions.md)
  network: false     # 是否連網
source: <built-in | external:<repo/url>>
pinned-ref: <外部來源填 commit/tag;內建填 n/a>
checksum: <外部來源填 sha256;內建填 n/a>
---

# <Skill 名稱>

<overview:這個 skill 做什麼、何時該用。>

## 步驟 / 用法

<高自由度用散文;高脆弱度用明確步驟。腳本放 scripts/,參考檔放 references/(離 SKILL.md 一層)。>
