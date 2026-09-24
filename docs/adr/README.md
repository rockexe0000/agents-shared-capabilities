# 架構決策記錄(ADR)

這個 catalog 的設計決策(WHY / 取捨)收錄在此。每份 ADR 是一份**自足的設計快照**:
說明某條機制「為什麼這樣設計」與踩過的雷,讀者不需存取任何外部 repo 即可理解。

> 這些 ADR 由團隊內部設計記錄複製而來(vendored),作為本 repo 的設計出處。
> 內部連結(handoff / knowledge / 跨 repo 路徑)在複製時已移除或改為本地 `docs/adr/` 連結。
> 內部逐步交接(handoff)不進本 repo。

| ADR | 主題 |
|-----|------|
| [0002](0002-agent-capability-provisioning.md) | Agent 能力供給機制(Skills + MCP);兩軸 enable/permissions、供應鏈、secret 參照、facade-default routing |
| [0005](0005-hook-capability-provisioning.md) | hook 納入供給(第三軸);canonical 事件字彙、授權前移(allow/deny) |
| [0006](0006-binary-dependency-provisioning.md) | skill 的 binary/CLI 依賴供給(第四軸);釘版本 + per-platform sha256 + provenance |
| [0007](0007-authorization-projection.md) | 執行授權投影;permissions 明確/自動模式、per-runtime enforcement config |
| [0008](0008-tooling-rust-unification.md) | 工具統一為單一可攜 Rust binary(`capsync`);ephemeral-init catalog、on-pod render |

編號從 0002 起(0000/0001/0003/0004 與本 catalog 無關,未收錄)。
