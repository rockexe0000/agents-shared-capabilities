---
title: "Runtime Projection — MCP matrix & the apply registry"
---

> Reference for **how a catalog MCP server reaches each Coding Agent runtime** — the two
> projection paths, the per-runtime config path + entry shape, and the DRY `apply` registry you
> extend to add a runtime. Design rationale for the tooling unification: [ADR 0008](adr/0008-tooling-rust-unification.md).
> The MCP *route* (facade vs direct) is orthogonal and covered in the root [README](../README.md#mcp-route預設-facade).

## Two projection paths

Both are `capsync` subcommands over the same catalog; they differ in *where* they run and *what*
they consume.

| path | command | runs | writes from | covers |
|------|---------|------|-------------|--------|
| **sync** (off-pod / dev) | `capsync sync` | a dev box with the runtimes installed | the catalog + the agent's `capabilities.md`, directly | the runtime config files it finds installed |
| **apply** (on-pod) | `capsync apply --from <dir>` | the pod, in `pre_boot` | the **render artifacts** a prior `capsync --render` wrote (`runtime-mcp.json` = the claude-shaped endpoint; `openab-agent-mcp.json` = the facade sources) | every runtime whose base dir exists on the pod |

`apply` is the path pods actually take (ADR 0008 Phase 4): an ephemeral init renders all axes into
`$HOME/.cap-render`, then one `capsync apply` projects them. It reshapes the single claude-shaped
`runtime-mcp.json` into each runtime's own config shape, gated so a runtime absent from the pod is
left untouched (no-op).

## MCP config matrix

The rendered `runtime-mcp.json` is **claude-shaped**: an http server is `{type:"http", url,
headers?}`, a stdio server is `{command, args, env}`. Each runtime's column below is what that entry
is reshaped into. `env:`/`${env:}` secret refs are preserved verbatim (resolved by the runtime or
the facade, never inlined).

| runtime | MCP config path | top-level key | remote (http) entry shape | sync | apply |
|---------|-----------------|---------------|---------------------------|:----:|:-----:|
| Claude Code | `~/.claude.json` | `mcpServers` | `{type:"http", url, headers?}` (identity) | ✅ | ✅ |
| Antigravity | `~/.gemini/config/mcp_config.json` | `mcpServers` | `{serverUrl, headers?}` | ✅ | ✅ |
| Codex | `~/.codex/config.toml` | `[mcp_servers.*]` (TOML managed block) | `url` + `http_headers` (literal) + `env_http_headers` (`${env:NAME}` refs) | ✅ | ✅ |
| opencode | `~/.config/opencode/opencode.json` | `mcp` | `{type:"remote", url, enabled:true, headers?}` | ✅ | ✅ |
| MiMo-Code | `~/.config/mimocode/mimocode.jsonc` | `mcp` | = opencode (fork, same shape) | — | ✅ |
| Cursor | `~/.cursor/mcp.json` | `mcpServers` | `{url, headers?}` (no `type`) | — | planned |
| Kiro | `~/.kiro/settings/mcp.json` | `mcpServers` | `{url, headers?}` (no `type`; `autoApprove`/`disabled` are the authz axis, not projected) | — | planned |
| Devin | `~/.config/devin/mcp_config.json` | `mcpServers` | `{url, transport:"http", headers?}` | — | planned |
| Kimi Code | `~/.kimi-code/mcp.json` | `mcpServers` | `{url, headers?}` (http = no transport; sse = `transport:"sse"`) | — | planned |
| Grok | `~/.grok/config.toml` | `[mcp_servers.*]` (TOML, ≈ codex) | TBC — medium confidence, verified when implemented | — | planned |
| Pi | — | — | — | — | N/A — Pi has no MCP support by design |

Legend: ✅ implemented · — not on this path · planned (Phase 1b, researched, not yet coded) · N/A.
stdio entries follow the same per-runtime rules (e.g. opencode uses `command: [cmd, ...args]` +
`environment`); the live facade server is http, so the http column is the hot path.

### Family grouping (shared writer ≠ shared shape)

The `mcpServers`-key runtimes share the **writer mechanism** (merge the reshaped entries into a
JSON `mcpServers` map) but **not the entry shape** — three distinct reshapes cover them:

- **identity** — Claude (keeps `type:"http"`).
- **strip `type`** → `{url, headers}` — Cursor, Kiro, Kimi.
- **`type`→`transport`** → `{url, transport:"http", headers}` — Devin.
- **`type`+`url`→`serverUrl`** → `{serverUrl, headers}` — Antigravity.

opencode/mimo are a separate `mcp`-key JSON shape; codex/grok are TOML managed blocks.

## The apply registry (the DRY seam)

On-pod MCP projection is table-driven in `tools-rs/src/main.rs`:

- **`McpApplyTarget`** — one row per `{src artifact, target path, writer, gate}`. `gate: Some(dir)`
  means "only apply if that runtime base dir exists on the pod"; `gate: None` is always-apply
  (facade sources + the claude endpoint).
- **`McpWriter`** — how a target writes:
  - `JsonMcpServers { key, reshape }` — merge the reshaped entries into the target JSON's `<key>`
    map (`mcpServers` for claude/antigravity/cursor/kiro/devin/kimi; `mcp` for opencode/mimo).
  - `CodexToml` — reconstruct a `[mcp_servers.*]` TOML managed block (codex; grok will reuse this
    family).
- **`mcp_apply_targets(home)`** returns the rows; **`apply_mcp`** iterates them (gate → writer).

### Adding a runtime

1. Add one `McpApplyTarget` row to `mcp_apply_targets` — its config path, `gate` = its base dir,
   and a `writer`.
2. Reuse an existing reshape/writer if the shape matches (see the family grouping); add a new
   `reshape` fn (a few lines, like `to_opencode_entry`) only for a genuinely new entry shape, or a
   new `McpWriter` variant only for a new *file* shape (e.g. a second TOML dialect).
3. Add a `parity.sh` assertion: seed the runtime's config dir, `apply`, assert the reshaped entry
   landed under the right key — plus a negative test that a pod lacking the dir is untouched.
4. Keep every existing runtime a no-op: the gate guarantees it, and `tools-rs-parity` enforces that
   the render golden stays byte-identical.

No render-side change is needed to add an `apply` target — `apply` reshapes the already-rendered
`runtime-mcp.json`, so existing agents' rendered artifacts (and their golden) are unaffected.
