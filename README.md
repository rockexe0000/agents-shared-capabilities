# agents-shared-capabilities

跨 Coding Agent 的**能力供給正本**:Agent Skills 與 MCP(Model Context Protocol)servers 的單一 catalog,投影進各 runtime(Claude Code、Codex、Antigravity、opencode…)。

> 設計與 WHY:ADR `agents-cold-memory:shared/adr/0002-agent-capability-provisioning`。
> 本 repo 只承載**集體 catalog**;個人專屬能力放該 agent 的 cold 命名空間 `agent-bot/{uid}/{skills,mcp}/`。

## 兩道閘

- **閘 1 存在(enable)**:`~/personal/capabilities.md`(per-agent,預設全關)決定裝/載入哪些 → `sync` 據此投影。
- **閘 2 授權(permissions)**:被接進來的 skill script / MCP tool 執行時,一律過 Hot Permission Boundary(`~/personal/permissions.md`)。

兩軸正交、疊加生效。載入過濾 ≠ shell 授權邊界。

**hook 是例外(第三軸,ADR 0005):** hook 由 harness 在事件上**自動執行**,不經 agent 的 tool call,**閘 2 對 hook 不 fire**。因此 hook 沒有三態 `ask`(fire 當下無互動點),只有 **allow / deny**——授權前移到「是否 enable」:寫在 `~/personal/capabilities.md` 的 `hooks: effect`(預設全關)。catalog PR = review 閘;外部 hook 需 vendored + 釘版本 + checksum。canonical 事件:`pre-tool | post-tool | session-start | stop | user-prompt-submit`,各 projector 映射到 host 原生,對應不到就 skip。

## Layout

```
skills/<name>/SKILL.md        # 集體 skill(frontmatter name+description;可帶 references/ scripts/ assets/;`requires:` 引用 bin/)
mcp/registry.yaml             # host-agnostic MCP server 定義(secret 只放參照)
hooks/registry.yaml           # host-agnostic hook 定義(canonical event + command;ADR 0005)
bin/registry.yaml             # host-agnostic binary/CLI 依賴定義(釘版本 + per-platform sha256;ADR 0006)
templates/                    # SKILL / mcp-server / hook / bin-tool / capabilities 範本
                              # (MCP `env:`/`${VAR}` 參照從 process.env 解;pod 由 k8s 注入,
                              #  local dev 自行 export。dotenv-file 後備已退場,ADR 0008 Decision 6)
tools/
  sync.sh / sync.js           # 讀 catalog + capabilities.md,投影進各 runtime
  lint.js                     # 驗 SKILL frontmatter、registry、capabilities 引用
  projectors/                 # 投影器
    claude-code.js            # direct:~/.claude/skills、~/.claude.json mcpServers、~/.claude/settings.json hooks
    codex.js                  # direct:~/.codex/skills、~/.codex/config.toml [mcp_servers]
    antigravity.js            # direct:~/.gemini/antigravity-cli/skills、~/.gemini/config/mcp_config.json
    oab-facade.js             # facade:~/.openab/agent/mcp.json(facade 背後的 source)
```

## MCP route(預設 facade)

registry 每個 server 標 `route`(預設 `facade`):

- **facade**(預設):藏在 OAB MCP Facade 後面的 source(寫進 openab `~/.openab/agent/mcp.json`)。agent runtime **只連 loopback facade、不持任何 key**;secret 以 `${env:VAR}` 由 openab 解析。所有 Coding Agent 走同一個 facade endpoint,新增 MCP 設一次、全 runtime 共用。
- **direct**:直接投影進各 runtime 的 MCP config。例外用途:① facade 本身(`oab-facade`)② 沒有 openab facade 的 host ③ facade 不能代理的 source。

## Binary 依賴(`bin/`,第四軸,ADR 0006)

skill 會 shell out 的 native binary/CLI 在 `bin/registry.yaml` 宣告一次(`name` / `source` / `pinned-version` / 每個 `(os-arch)` 的 `asset`+`sha256`〔+選用 `provenance`〕);skill 於 `SKILL.md` frontmatter 用 `requires: [{name, min}]` 引用,`min` 是版本地板。

- **供應鏈**:外部來源一律 pinned-version + per-platform sha256(`lint.js` 強制);`provenance: none|attestation|cosign`(有就驗、`none` 為顯性降級)。安裝/執行外部 binary 過 permissions 的 `ask` 閘。
- **安裝(dev,已實作)**:`node tools/sync.js --with-tools`(opt-in)算 enabled skills 的 required-bins 閉包 → 抓釘版 asset → 驗 sha256 → 落受管 bin dir(`~/.agents-shared-capabilities/state/bin`)並提示掛 PATH;冪等(已裝且 checksum 相符則 skip),不再 required 的自動 GC。`node tools/sync.js --check-tools [--capabilities <f>]` 驗安裝漂移(present/版本/checksum,漂移 exit 1,供 CI/cron)。
- **安裝(pod,規劃中)**:agents-infra build-time 讀同一 manifest 烤進 image。實作進度見 handoff `agents-cold-memory:shared/handoffs/discord-1546431897800933426-binary-dependency-provisioning`。
- **lint**:`requires` 必須 resolve 到 `bin/registry.yaml`,且釘版 ≥ 各 requiring skill 的 `min`。

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

## 漂移檢查(drift check)

option-C(render → configMap)下,pod 上沒有 repo checkout,漂移風險是**已 commit 的 render 產物** vs **catalog + 該 agent `capabilities.md`** 走鐘。`--check` 是確定性比對:重跑 render 與指定目錄的已 commit 產物比較,漂移則 exit 1(供 CI 或維運 cron fail loud)。

```sh
node tools/sync.js --check <committed-artifacts-dir> --capabilities <capabilities.md>
```

例:CI 於 agents-infra checkout 本 repo + cold repo,對每個 agent overlay 跑 `--check overlays/<agent> --capabilities <cold>/agent-bot/<agent>/personal/capabilities.md`。
