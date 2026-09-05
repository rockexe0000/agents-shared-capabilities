# agents-shared-capabilities

跨 Coding Agent 的**能力供給正本**:Agent Skills 與 MCP(Model Context Protocol)servers 的單一 catalog,投影進各 runtime(Claude Code、Codex、Antigravity、opencode…)。

> 設計與 WHY:ADR `agents-cold-memory:shared/adr/0002-agent-capability-provisioning`。
> 本 repo 只承載**集體 catalog**;個人專屬能力放該 agent 的 cold 命名空間 `agent-bot/{uid}/{skills,mcp}/`。

## 兩道閘

- **閘 1 存在(enable)**:`~/personal/capabilities.md`(per-agent,預設全關)決定裝/載入哪些 → `sync` 據此投影。
- **閘 2 授權(permissions)**:被接進來的 skill script / MCP tool 執行時,一律過 Hot Permission Boundary(`~/personal/permissions.md`)。

兩軸正交、疊加生效。載入過濾 ≠ shell 授權邊界。

## Layout

```
skills/<name>/SKILL.md        # 集體 skill(frontmatter name+description;可帶 references/ scripts/ assets/)
mcp/registry.yaml             # host-agnostic MCP server 定義(secret 只放參照)
templates/                    # SKILL / mcp-server / capabilities 範本
secrets/.env.example          # MCP 需要的環境變數「名稱」清單;真值放本地 secrets/.env(gitignore)
tools/
  sync.sh / sync.js           # 讀 catalog + capabilities.md,投影進各 runtime
  lint.js                     # 驗 SKILL frontmatter、registry、capabilities 引用
  projectors/                 # 每個 runtime 一支投影器
    claude-code.js            # ~/.claude/skills、~/.claude.json mcpServers
    codex.js                  # ~/.codex/skills、~/.codex/config.toml [mcp_servers]
    antigravity.js            # ~/.gemini/antigravity-cli/skills、~/.gemini/config/mcp_config.json
```

## 用法

```sh
# 1. clone 到固定位置
git clone https://github.com/rockexe0000/agents-shared-capabilities ~/.agents-shared-capabilities

# 2. 從範本生 per-agent 選擇清單(本地,cold 版控於 agent-bot/{uid}/personal)
cp ~/.agents-shared-capabilities/templates/capabilities.template.md ~/personal/capabilities.md
# 編輯 ~/personal/capabilities.md:列出要啟用的 skill 與 MCP server(預設全關)

# 3. 投影進本機已安裝的每個 runtime
node ~/.agents-shared-capabilities/tools/sync.js        # 或 tools/sync.sh

# 4. lint(CI 也會跑)
node ~/.agents-shared-capabilities/tools/lint.js
```

Codex 需重啟才吃到新 skill;Claude Code 下次啟動載入。

## 邊界

- **憑證不進 repo**:registry 只放 secret 參照(`env:` / `op://` / `vault:`),真值由 resolver 於本地取;OAuth token 交各 runtime 本地存。
- **外部能力**:先審 + 釘版本 + vendored(PR merge = review),不 live-link 遠端 marketplace。
