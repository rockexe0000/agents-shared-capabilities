---
title: "Capabilities Schema"
source: owner-directed capability provisioning mechanism, Discord thread 1544977892868620378; design WHY in cold ADR agents-cold-memory:shared/adr/0002-agent-capability-provisioning. Relocated here from agents-shared-memory:shared/capabilities-schema.md 2026-10-01 — the removal commit (13f18eb) claimed the migration but the file never landed in this catalog repo, leaving every agent's capabilities.md cite (agents-shared-capabilities:docs/capabilities-schema.md) dangling; this restores it at the cited path.
scope: shared schema for an agent's personal `capabilities.md` (the per-agent enable-list — which catalog skills / MCP servers this agent turns on)
timestamp: "2026-09-05T00:00:00Z"
status: active
layer: warm
trigger: Load when creating, editing, or validating a personal `capabilities.md` enable-list.
---

# Capabilities Schema

`capabilities.md` is the per-agent, **host-agnostic enable-list**: which catalog skills,
MCP servers, and hooks this agent turns on. It is the *selection* axis, **orthogonal to authorization** —
`permissions.md` decides whether an enabled capability may *run*; `capabilities.md` decides only
whether it is *installed/loaded*. `tools/sync.js` (in `agents-shared-capabilities`) reads this file
and projects the enabled subset into each installed runtime. Default is **all-off**: only what is
listed here is projected. Design rationale (why the two axes are split): cold ADR
`agents-cold-memory:shared/adr/0002-agent-capability-provisioning`.

The concrete file holds this agent's selections and is **personal**, at `personal/capabilities.md`
(storage/versioning: `~/shared/memory-architecture.md` → Storage Locations); this schema lives in
`shared/` so any agent can author its own. A `capabilities.md` cites this schema via `cite_ref`.

What `capabilities.md` is **not**:
- Not the **authorization**. "May this skill's script / this MCP tool actually run, and to what
  scope" lives in `permissions.md` + the Hot Permission Boundary (`AGENTS.md`). Enabling ≠ allowing.
- Not the **catalog**. The skill/MCP *definitions* live in `agents-shared-capabilities`
  (`skills/`, `mcp/registry.yaml`) or the agent's own cold namespace (`agent-bot/{uid}/{skills,mcp}/`);
  this file only *references* them by name.
- Not the **secrets**. MCP secrets are references resolved locally at sync time; never inline here.

## Structure

This block is the enforced structural contract (`conformance` tag): every `capabilities.md` citing
this schema is checked by `tools/lint-schema.js` against the keys marked `REQUIRED` (type and enum
derived from each placeholder). Names must match a catalog/personal skill folder or an
`mcp/registry.yaml` server; that cross-file existence check is done by `sync.js`, not this schema.

```yaml conformance
skills:
  - name: ""                # REQUIRED - skill name (catalog folder, or personal skill)
    source: "catalog|personal"  # REQUIRED - where the skill is sourced from
mcp:
  servers:
    - name: ""              # REQUIRED - MCP server name from mcp/registry.yaml (or personal)
      tools: []             # OPTIONAL - allowed tool-name globs; omit or ["*"] = all
hooks:
  - name: ""                # REQUIRED - hook name from hooks/registry.yaml (or personal)
    effect: "allow|deny"    # REQUIRED - allow = project + auto-run; deny = off. NO `ask` (see Rules / ADR 0005)
```

## Rules

- **Default all-off.** An unlisted skill/server is not projected. Removing an entry and re-running
  `sync` stops projecting it (runtimes may need a reload / manual removal of a stale entry).
- **`source`** selects the catalog (`agents-shared-capabilities`) vs this agent's own cold namespace
  (`agent-bot/{uid}/`); on a name collision, **personal overrides catalog** (enforced by `sync.js`).
- **`tools`** narrows an MCP server's exposed tools (per-scope收斂, mirroring Antigravity/opencode);
  omit or `["*"]` to expose all. It is a *loading/visibility* filter, **not** an authorization gate.
- **`hooks` are the exception to the selection/authorization split (ADR 0005).** A hook auto-executes
  on a runtime event and **bypasses the Permission Boundary** (it is not an agent tool call), so for a
  hook *enabling ≈ authorizing auto-execution*. Its `effect` is therefore **`allow` / `deny` only — no
  `ask`** (there is no fire-time interaction point to prompt at). Authorization moves forward to this
  enable decision (verified-owner, via catalog PR review); default all-off. Each name resolves to a
  `hooks/registry.yaml` entry (canonical event + command); external hooks additionally need vendored +
  pinned + checksummed catalog entries.
- Enabling an **external** (third-party) skill/server is a permission-gated action — see
  `permissions.md` / Hot Permission Boundary; this file records the selection, not the approval.
- Keep the concrete file at `personal/capabilities.md` (per-agent); keep this schema generic.
