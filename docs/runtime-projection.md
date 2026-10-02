---
title: "Runtime Projection — sync vs apply, and the five axes"
---

> How a catalog capability reaches each Coding Agent runtime: the **two projection paths**
> (`sync` off-pod, `apply` on-pod) and, for each of the **five axes** (authz / mcp / skills / bin /
> hooks), the per-runtime target, shape, and current coverage. Tooling-unification rationale:
> [ADR 0008](adr/0008-tooling-rust-unification.md). Per-axis design ADRs:
> [0002](adr/0002-agent-capability-provisioning.md) (skills+mcp) ·
> [0005](adr/0005-hook-capability-provisioning.md) (hooks) ·
> [0006](adr/0006-binary-dependency-provisioning.md) (bin) ·
> [0007](adr/0007-authorization-projection.md) (authz). The MCP *route* (facade vs direct) is
> orthogonal and lives in the root [README](../README.md#mcp-route預設-facade).

## Two projection paths: `sync` vs `apply`

Both are `capsync` subcommands over the same catalog. They differ in **where they run** and **what
they consume**.

- **`sync`** (off-pod / dev) — reads the catalog + the agent's `capabilities.md` **live**, computes
  each runtime's config, and writes it straight into `$HOME`. One shot: compute **and** write.
- **`apply --from <dir>`** (on-pod) — the pod has **no catalog checkout**. It consumes a bundle of
  **pre-rendered artifacts** (`capsync --render` wrote them to `$HOME/.cap-render`) and projects
  those into each runtime. It is the path pods actually take, in `pre_boot`.

| | `sync` (off-pod) | `apply` (on-pod) |
|---|---|---|
| runs | dev box, manual / opt-in | pod `pre_boot`, automatic |
| input | catalog + `capabilities.md` (live) | render artifacts (`~/.cap-render`) |
| MCP data form | `ResolvedServer` (structured: transport/url/headers/command/env) | already-rendered **claude-shaped JSON** |
| axes | skills · mcp · hooks (· bin with `--with-tools`) | authz · mcp · skills · bin (**no hooks yet**) |
| per-runtime gate | each projector checks the runtime is installed | `gate` = the runtime's base dir exists |

### The pipeline (the `render` step in the middle)

```
dev :  catalog + capabilities.md  ──sync───────────────────▶  runtime configs
pod :  catalog + capabilities.md  ──render──▶ ~/.cap-render ──apply──▶  runtime configs
          (ephemeral init, has catalog)        (artifacts)    (pre_boot, no catalog)
```

`sync` fuses *compute* and *write*. The pod splits them: an **ephemeral init** (catalog cloned at a
pinned ref) runs `--render` to compute every axis into `~/.cap-render`; the main container's
`pre_boot` then runs `apply`, which needs **only the artifacts** — no catalog, no toolchain, and no
network for the local axes.

### Why split (ADR 0008 option-C)

1. **Clean pod image** — the main container carries no catalog and no Rust toolchain.
2. **Single source of truth + drift check** — `capsync --check <dir>` deterministically re-renders
   and diffs the committed artifacts, so CI/cron catch catalog-vs-artifact drift (fail loud).
3. **Fail-closed authz** — the init's render assertion is the security gate; `apply` runs authz
   first and each axis independently, so a later (network) bin hiccup never skips the security axis.

### Why some shape logic appears twice

A runtime's "reshape to its native config" exists in **two forms** because the two paths hold
different inputs:

- `sync` has a `ResolvedServer` (transport/url/headers are separate fields) → it builds the native
  entry directly (`claude_entry`, `opencode_entry`, …).
- `apply` has only the **rendered claude-shaped JSON** (the catalog detail was flattened at render
  time) → it does a **JSON→JSON reshape** (`to_opencode_entry`, `apply_codex_mcp`, …).

They produce the **same target shape** by different means. The multi-runtime work (opencode/mimo,
and next cursor/kiro/devin/kimi/grok) extends the **`apply`** side — the path pods take — so it is
JSON reshape + a per-runtime gate.

## The five axes at a glance

| axis | render artifact(s) | `sync` | `apply` | runtimes today |
|------|--------------------|:------:|:-------:|----------------|
| **authz** | `authz-<rt>.json` (+`authz-suggest.txt`) | — | ✅ | claude-code, antigravity |
| **mcp** | `runtime-mcp.json`, `openab-agent-mcp.json` | ✅ | ✅ | sync: claude/codex/antigravity/opencode · apply: + mimo/cursor/kiro/kimi/devin/grok (+ facade); pi N/A |
| **skills** | `skills.tar.b64`, `skills.list` | ✅ per-runtime | ✅ single dir | sync: claude/codex/antigravity/opencode |
| **bin** | `bin-install.tsv` | ✅ `--with-tools` | ✅ | runtime-agnostic (`~/bin` on PATH) |
| **hooks** | — (not rendered yet) | ✅ | — (Phase 4) | claude-code, antigravity |

Legend: ✅ implemented · — not on this path. Each axis's projector **gates on the runtime being
present** (base dir exists) → a runtime absent from a pod is a no-op.

---

### authz (ADR 0007)

Projects the agent's permission decisions into each runtime's settings, from the rendered
`authz-<runtime>.json` (`{allow, deny}`). **`apply`-only** (`sync` leaves a dev's own runtime
permissions alone). Merge semantics (RMW + `.bak`, mirroring `perms-apply.sh`): `permissions.allow`
is **replaced** (projection owns the allowlist → GC-correct), `permissions.deny` is the **union** of
existing ∪ managed (deny never auto-drops); every other setting is preserved.

| runtime | settings file | scope shape |
|---------|---------------|-------------|
| Claude Code | `~/.claude/settings.json` | `Tool(spec)` / MCP `Server/tool` (`wrap_claude` / `wrap_claude_mcp`) |
| Antigravity | `~/.gemini/antigravity-cli/settings.json` | `action(target)` (`wrap_antigravity`) |

Planned (Phase 3): the list-model runtimes (cursor / opencode / kimi / kiro / grok) and codex's
non-list `approval_policy` + `sandbox_mode`; pi has no authz model. `autoApprove` (kiro) and other
per-tool fields belong to this axis, not MCP.

### mcp (ADR 0002)

The rendered `runtime-mcp.json` is **claude-shaped** (http → `{type:"http", url, headers?}`, stdio →
`{command, args, env}`); each runtime's column is what that entry is reshaped into. `env:`/`${env:}`
secret refs are preserved verbatim (resolved by the runtime or the facade, never inlined). The
facade sources go to `~/.openab/agent/mcp.json`; the facade endpoint is the one claude-shaped server
every runtime connects to.

| runtime | MCP config path | key | remote (http) entry shape | sync | apply |
|---------|-----------------|-----|---------------------------|:----:|:-----:|
| Claude Code | `~/.claude.json` | `mcpServers` | `{type:"http", url, headers?}` (identity) | ✅ | ✅ |
| Antigravity | `~/.gemini/config/mcp_config.json` | `mcpServers` | `{serverUrl, headers?}` | ✅ | ✅ |
| Codex | `~/.codex/config.toml` | `[mcp_servers.*]` TOML | `url` + `http_headers` + `env_http_headers` (`${env:NAME}`) | ✅ | ✅ |
| opencode | `~/.config/opencode/opencode.json` | `mcp` | `{type:"remote", url, enabled:true, headers?}` | ✅ | ✅ |
| MiMo-Code | `~/.config/mimocode/mimocode.jsonc` | `mcp` | = opencode (fork) | — | ✅ |
| Cursor | `~/.cursor/mcp.json` | `mcpServers` | `{url, headers?}` (no `type`) | — | ✅ |
| Kiro | `~/.kiro/settings/mcp.json` | `mcpServers` | `{url, headers?}` (no `type`) | — | ✅ |
| Devin | `~/.config/devin/mcp_config.json` | `mcpServers` | `{url, transport:"http", headers?}` | — | ✅ |
| Kimi Code | `~/.kimi-code/mcp.json` | `mcpServers` | `{url, headers?}` (http = no transport; sse = `transport:"sse"`) | — | ✅ |
| Grok | `~/.grok/config.toml` | `[mcp_servers.*]` TOML | `url` + single inline `headers = {…}` (`${env:NAME}`→grok's `${NAME}`) | — | ✅¹ |
| Pi | — | — | — | — | N/A — no MCP by design |

¹ Grok header-value interpolation (`${VAR}`) is medium-confidence (official docs show the
`headers` table + `${VAR}` env syntax but don't spell out header interpolation); no live grok
runtime exists yet, so this is forward-looking + gated no-op until confirmed on a grok pod.

**Family grouping (shared writer ≠ shared shape).** The `mcpServers`-key runtimes share the
*writer* (merge reshaped entries into a `mcpServers` map) but need **four distinct reshapes**:
identity (claude, keeps `type:"http"`) · strip `type` → `{url, headers}` (cursor / kiro / kimi) ·
`type`→`transport:"http"` (devin) · `type`+`url`→`serverUrl` (antigravity). opencode/mimo are a
separate `mcp`-key JSON; codex/grok are TOML managed blocks (different shapes: codex splits
`http_headers`/`env_http_headers`, grok uses one inline `headers` table).

**The `apply` MCP registry (the DRY seam)** — table-driven in `tools-rs/src/main.rs`:

- `McpApplyTarget` — one row per `{src artifact, target path, writer, gate}`. `gate: None` =
  always-apply (facade sources + the claude endpoint); `gate: Some(dir)` = only if that runtime
  base dir exists.
- `McpWriter` — `JsonMcpServers { key, reshape }` (merge into the target JSON's `<key>` map) or
  `CodexToml` / `GrokToml` (each reconstructs a `[mcp_servers.*]` TOML managed block — codex
  splits `http_headers`/`env_http_headers`, grok uses one inline `headers` table).
- `mcp_apply_targets(home)` returns the rows; `apply_mcp` iterates them (gate → writer).

*Adding a runtime:* add one row; reuse a reshape/writer if the shape matches, else add a small
`reshape` fn (like `to_opencode_entry`) for a new entry shape or a `McpWriter` variant for a new
file shape; add a `parity.sh` assertion (+ a negative no-op test). No render-side change is needed —
`apply` reshapes the already-rendered `runtime-mcp.json`, so existing agents' golden is unaffected.

### skills (ADR 0002)

Rendered as `skills.tar.b64` + `skills.list` (the enabled skills' `SKILL.md` trees). Each runtime
treats its skills dir as "installed" when its base dir exists.

- **`sync`** — per-runtime **symlinks** (`SKILL_RUNTIMES`): Claude `~/.claude/skills`, Codex
  `~/.codex/skills`, Antigravity `~/.gemini/antigravity-cli/skills`, opencode
  `~/.config/opencode/skills`.
- **`apply`** — extracts the tar into a **single** `SKILLS_DIR` (default `~/.claude/skills`; env
  override), GC'd against a `.catalog-managed` marker.

> ⚠️ **Known gaps (Phase 2):** `apply` skills is single-dir, not per-runtime like `sync`; and the
> Codex skills path should be `~/.agents/skills` (official), not `~/.codex/skills`. Both are tracked
> for the per-runtime skills pass.

### bin (ADR 0006)

Rendered as `bin-install.tsv` (the pinned CLI closure of enabled skills' `requires`). Both
`sync --with-tools` and `apply` fetch → verify sha256 → extract into the install dir
(`BIN_INSTALL_DIR`, else `~/.local/bin` if on PATH), lockfile-idempotent, GC'ing no-longer-required
tools. **Runtime-agnostic**: there is no per-runtime bin mechanism — every runtime just needs the
managed bin dir on PATH. This is the one axis that is a single shared implementation.

### hooks (ADR 0005)

Canonical events (`CANONICAL_HOOK_EVENTS`): `pre-tool`, `post-tool`, `session-start`, `stop`,
`user-prompt-submit`, mapped to each runtime's native event name (unmapped → skip). Only
`effect: allow` hooks are projected; the projector rewrites a managed block (strip prior, write
fresh) so a now-disabled hook is removed.

- **`sync`** — Claude `~/.claude/settings.json` (`hooks`) via `project_claude_hooks`; Antigravity
  `~/.gemini/config/hooks.json` via `project_antigravity_hooks`.
- **`apply`** — **not yet**: hooks are not among the rendered artifacts, and `apply_cmd` runs only
  authz/mcp/skills/bin. **Phase 4** adds a hook render artifact + an `apply_hooks` axis (and the
  registry-driven per-runtime projectors for the other hook-capable runtimes: codex / cursor / kimi
  settings-hooks, kiro `.kiro/hooks/*.json`; opencode/mimo/pi are plugin-only → N/A).
