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
  projectors/                 # 投影器
    claude-code.js            # direct:~/.claude/skills、~/.claude.json mcpServers
    codex.js                  # direct:~/.codex/skills、~/.codex/config.toml [mcp_servers]
    antigravity.js            # direct:~/.gemini/antigravity-cli/skills、~/.gemini/config/mcp_config.json
    oab-facade.js             # facade:~/.openab/agent/mcp.json(facade 背後的 source)
```

## MCP route(預設 facade)

registry 每個 server 標 `route`(預設 `facade`):

- **facade**(預設):藏在 OAB MCP Facade 後面的 source(寫進 openab `~/.openab/agent/mcp.json`)。agent runtime **只連 loopback facade、不持任何 key**;secret 以 `${env:VAR}` 由 openab 解析。所有 Coding Agent 走同一個 facade endpoint,新增 MCP 設一次、全 runtime 共用。
- **direct**:直接投影進各 runtime 的 MCP config。例外用途:① facade 本身(`oab-facade`)② 沒有 openab facade 的 host ③ facade 不能代理的 source。

## 用法

### 一鍵 bootstrap(建議)

冪等接入:clone/pull 三正本 → 佈 `~/` symlink → 首次從範本生 `capabilities.md` → sync → 印 reload 提示。可安全重跑(不覆蓋既有 `capabilities.md`)。

```sh
AGENT_UID=<uid> ~/.agents-shared-capabilities/bootstrap.sh
```

repo URL / checkout 路徑 / `NO_SYNC` / `NO_PULL` 皆可用 env 覆蓋(見 `bootstrap.sh` 檔頭)。維運面用 OpenAB config cron 週期跑 `sync` + `lint` 做漂移檢查。

### 手動分解

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
