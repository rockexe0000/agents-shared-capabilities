'use strict';
/*
 * claude-code projector — merge enabled MCP servers into ~/.claude.json `mcpServers`.
 * SAFE read-modify-write: JSON round-trip (lossless), .bak backup, only add/update
 * our server keys by name (never rewrites unrelated keys, never auto-removes).
 */
const fs = require('fs');
const path = require('path');

function target(home) { return path.join(home, '.claude.json'); }
function installed(home) { return fs.existsSync(path.join(home, '.claude')) || fs.existsSync(target(home)); }

// ---- hooks (ADR 0005) ----
function hookTarget(home) { return path.join(home, '.claude', 'settings.json'); }
const MANAGED = 'agents-shared-capabilities'; // marker to identify our own hook entries
// canonical event -> Claude Code native hook event
const HOOK_EVENT = {
  'pre-tool': 'PreToolUse',
  'post-tool': 'PostToolUse',
  'session-start': 'SessionStart',
  'stop': 'Stop',
  'user-prompt-submit': 'UserPromptSubmit',
};

/**
 * Project enabled hooks into ~/.claude/settings.json `hooks`. Idempotent:
 * strips every entry we previously wrote (marked `_managedBy`) then re-adds the
 * currently-enabled set, so a disabled hook disappears and user-authored hooks
 * (no marker) are never touched. Unmappable canonical events are skipped + logged.
 * @param {Array} hooks resolved hook defs [{name,event,matcher,command}]
 * @returns {{updated:string[], skipped:Array<{name:string,event:string}>, target:string}}
 */
function projectHooks(hooks, home, opts = {}) {
  const file = hookTarget(home);
  const exists = fs.existsSync(file);
  // nothing to add and no file to prune → don't create an empty settings.json
  if (!exists && hooks.length === 0) return { updated: [], skipped: [], target: file };
  let cfg = {};
  if (exists) {
    cfg = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!opts.dry) fs.copyFileSync(file, file + '.bak');
  }
  cfg.hooks = cfg.hooks || {};
  // 1. strip our previously-managed entries
  for (const ev of Object.keys(cfg.hooks)) {
    const groups = (cfg.hooks[ev] || [])
      .map((g) => Object.assign({}, g, { hooks: (g.hooks || []).filter((h) => h._managedBy !== MANAGED) }))
      .filter((g) => (g.hooks || []).length);
    if (groups.length) cfg.hooks[ev] = groups; else delete cfg.hooks[ev];
  }
  // 2. add currently-enabled hooks
  const updated = [];
  const skipped = [];
  for (const h of hooks) {
    const native = HOOK_EVENT[h.event];
    if (!native) { skipped.push({ name: h.name, event: h.event }); continue; }
    const entry = { type: 'command', command: h.command, _managedBy: MANAGED, _hook: h.name };
    const group = h.matcher ? { matcher: h.matcher, hooks: [entry] } : { hooks: [entry] };
    (cfg.hooks[native] = cfg.hooks[native] || []).push(group);
    updated.push(h.name);
  }
  if (Object.keys(cfg.hooks).length === 0) delete cfg.hooks;
  if (!opts.dry) fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  return { updated, skipped, target: file };
}

/**
 * @param {Array} servers resolved server defs
 * @param {string} home
 * @param {{dry?:boolean}} opts
 * @returns {{updated:string[], target:string}}
 */
function projectMcp(servers, home, opts = {}) {
  const file = target(home);
  let cfg = {};
  if (fs.existsSync(file)) {
    cfg = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!opts.dry) fs.copyFileSync(file, file + '.bak');
  }
  cfg.mcpServers = cfg.mcpServers || {};
  const updated = [];
  for (const s of servers) {
    const entry = s.transport === 'http'
      ? Object.assign({ type: 'http', url: s.url }, Object.keys(s.headers || {}).length ? { headers: s.headers } : {})
      : { command: s.command, args: s.args || [], env: s.env || {} };
    cfg.mcpServers[s.name] = entry;
    updated.push(s.name);
  }
  if (!opts.dry) fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  return { updated, target: file };
}

module.exports = { id: 'claude-code', installed, projectMcp, projectHooks };
