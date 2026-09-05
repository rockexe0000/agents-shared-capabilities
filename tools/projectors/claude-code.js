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
      ? { type: 'http', url: s.url }
      : { command: s.command, args: s.args || [], env: s.env || {} };
    cfg.mcpServers[s.name] = entry;
    updated.push(s.name);
  }
  if (!opts.dry) fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  return { updated, target: file };
}

module.exports = { id: 'claude-code', installed, projectMcp };
