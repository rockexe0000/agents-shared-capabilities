---
title: "Capabilities"
source: "<platform:thread-id 授權來源>"
scope: "this agent's capability enable-list — 決定 sync 投影哪些 skill/MCP;預設全關"
timestamp: "<ISO8601>"
status: active
layer: warm
trigger: "Load when syncing/選擇 this agent 要啟用哪些 skill 或 MCP server。"
---

# Capabilities
# per-agent 選擇清單(enable)。與授權正交:此檔決定「裝/載入哪些」,
# permissions.md 決定「准不准跑」。預設全關 —— 只有列在下方的才會被 sync 投影。
# 個人專屬能力(非 catalog)以 source: personal 標示,來源為 agent-bot/{uid}/{skills,mcp}/。
# schema:agents-shared-capabilities:docs/capabilities-schema.md(cite_ref 為選用,不指向 memory repo)。

skills:
  - name: hello-capability
    source: catalog        # catalog | personal

mcp:
  servers: []              # 例:- { name: example-fs, tools: ["*"] }

hooks:
  # 例:- { name: post-edit-noop, effect: allow }
  # hook 由 harness 自動執行、繞過 Permission Boundary → 只有 allow/deny、無 ask;
  # 預設全關,設 allow 是需 verified-owner 授權的決定(ADR 0005)。
