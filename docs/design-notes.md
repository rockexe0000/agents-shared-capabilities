# 設計出處與內部參照(design notes / internal references)

這個 catalog 的**設計決策(WHY / 取捨)**與**實作出處**記錄在團隊內部 memory
(ADR 與 handoff)。集中列在這裡,讓頂層 `README` / `CONTRIBUTING` / `tools-rs/README`
保持精簡,不把內部路徑散落各處。

> 這些是**內部** memory repo 的參照(`agents-cold-memory:` / `agents-shared-memory:`);
> 只有能存取該 memory 的維護者能開啟,對外部讀者是**不透明指標**。本 catalog 的行為
> 由 repo 內的 README / registry / SKILL 自足描述,不需讀內部 memory 也能使用。

## ADR(設計與 WHY)

| ADR | 主題 | 出處 |
|-----|------|------|
| 0002 | 能力供給機制(skills + MCP catalog、兩軸 enable/permissions、供應鏈、secret 參照) | `agents-cold-memory:shared/adr/0002-agent-capability-provisioning` |
| 0005 | hook 軸(第三軸;harness 自動執行、繞過 Permission Boundary → 只有 allow/deny) | `agents-cold-memory:shared/adr/0005-hook-capability-provisioning` |
| 0006 | binary/CLI 依賴軸(第四軸;釘版本 + per-platform sha256 + provenance) | `agents-cold-memory:shared/adr/0006-binary-dependency-provisioning` |
| 0007 | 授權投影(execution-authorization、`authorize_skill_requires`) | `agents-cold-memory:shared/adr/0007-authorization-projection` |
| 0008 | 工具 Rust 統一(`capsync` 單一 static binary,取代 node + sh 後端) | `agents-cold-memory:shared/adr/0008-tooling-rust-unification` |

## Handoff(實作進度 / work-list)

| 主題 | 出處 |
|------|------|
| binary 依賴供給(pod build-time 供給規劃進度) | `agents-cold-memory:shared/handoffs/discord-1546431897800933426-binary-dependency-provisioning` |
| 工具 Rust 統一(capsync 遷移 work-list) | `agents-cold-memory:shared/handoffs/discord-1548114275158200351-tooling-rust-unification` |

## Schema(memory-note 範本引用)

`templates/capabilities.template.md` 產出的是一份會落進 agent 個人 memory 命名空間的
warm note,其 front-matter `cite_ref` 指向該 note 需符合的 schema:

- `agents-shared-memory:shared/capabilities-schema.md`

此 `cite_ref` 是**範本產物的功能性 metadata**(讓複製出的 `~/personal/capabilities.md`
符合 memory schema),不是散落在文件裡的說明性連結,因此保留在範本 front-matter 中。
