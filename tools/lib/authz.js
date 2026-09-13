'use strict';
/*
 * authz.js — execution-authorization projection (閘 2), ADR 0007.
 *
 * Turns an agent's authorization intent into the per-runtime allow/deny lists
 * that each Coding Agent runtime's native permission config enforces. This is the
 * fourth projection axis (after skills / MCP / bin): enable ≠ authorize — a skill
 * can be installed yet its commands still gated until authorized here.
 *
 * Sources (ADR 0007 Decisions 2–4):
 *   B (default, `authorize_skill_requires: explicit`): the allowlist is ONLY the
 *     commands permissions.md explicitly `allow`s (operation `run command`,
 *     scope = command). `requires` is merely a copy-paste suggestion list.
 *   A (`authorize_skill_requires: auto`): additionally derive allows from the
 *     enabled skills' `requires` (each resolved to its bin tool's command).
 *   deny ALWAYS overrides (deny > ask > allow) — a permissions.md deny removes the
 *     command from allow regardless of mode; `ask` commands are left unprojected so
 *     the runtime prompts.
 *
 * Per-runtime mapping (Decision 5): one abstract "allow command X" → each runtime's
 * native token. New runtime = one more entry in RUNTIME_AUTHZ.
 */
const { parsePermissions } = require('./parse');
const { loadBinTools, requiredToolNames } = require('./install-bin');

// abstract command X → runtime-native permission token. agy uses action(target)
// (`command(cfdrop)` = cfdrop with any args); Claude Code uses Bash(cmd:*).
const RUNTIME_AUTHZ = {
  antigravity: (cmd) => `command(${cmd})`,
  'claude-code': (cmd) => `Bash(${cmd}:*)`,
};
const AUTHZ_RUNTIMES = Object.keys(RUNTIME_AUTHZ);

// enabled skills' `requires` → [{ tool, command }] suggestion list. Each required
// bin-tool name resolves to its `bin` (the on-PATH executable = the command a
// runtime gates); unknown tools fall back to the required name itself.
function requiredCommands(enable, REPO, base) {
  const tools = loadBinTools(REPO, base);
  return requiredToolNames(enable, REPO, base)
    .map((name) => ({ tool: name, command: (tools.get(name) || {}).bin || name }))
    .sort((a, b) => a.command.localeCompare(b.command));
}

/**
 * Compute the runtime-agnostic authorization from intent + enable-list.
 * @returns {{ flag, allow:string[], deny:string[], suggestions:[{tool,command}] }}
 *   allow/deny are sorted, de-duped command names; deny has already been removed
 *   from allow.
 */
function buildAuthz(enable, permsText, REPO, base) {
  const perms = parsePermissions(permsText || '');
  const suggestions = requiredCommands(enable, REPO, base);
  const allow = new Set(perms.allow);
  if (perms.flag === 'auto') for (const s of suggestions) allow.add(s.command);
  const deny = new Set(perms.deny);
  for (const d of deny) allow.delete(d); // deny > allow
  return {
    flag: perms.flag,
    allow: [...allow].sort(),
    deny: [...deny].sort(),
    suggestions,
  };
}

/**
 * Map runtime-agnostic authz → one runtime's native allow/deny token arrays.
 * @returns {{ allow:string[], deny:string[] }}
 */
function mapAuthz(runtimeId, authz) {
  const f = RUNTIME_AUTHZ[runtimeId];
  if (!f) throw new Error(`unknown authz runtime: ${runtimeId}`);
  return { allow: authz.allow.map(f), deny: authz.deny.map(f) };
}

// human-readable copy-paste hint: the `requires`-derived command suggestions and,
// under explicit mode, which are not yet granted (so owner can paste them into
// permissions.md). Deterministic text for --check.
function suggestText(authz) {
  const lines = [];
  lines.push(`# authorize_skill_requires: ${authz.flag}`);
  if (!authz.suggestions.length) {
    lines.push('# (no enabled skill declares a `requires` command)');
  } else {
    lines.push('# commands enabled skills require (paste an allow rule into permissions.md to grant):');
    for (const s of authz.suggestions) {
      const granted = authz.allow.includes(s.command);
      lines.push(`#   ${s.command}  (from ${s.tool})${granted ? '  [granted]' : ''}`);
    }
  }
  return lines.join('\n') + '\n';
}

module.exports = { buildAuthz, mapAuthz, requiredCommands, suggestText, RUNTIME_AUTHZ, AUTHZ_RUNTIMES };
