'use strict';
/*
 * antigravity (CLI) projector — merge enabled MCP servers into
 * ~/.gemini/config/mcp_config.json `mcpServers`.
 * SAFE read-modify-write: lossless JSON round-trip, .bak backup, only add/update
 * our server keys by name. Antigravity uses `serverUrl` (+ optional `headers`)
 * for remote transport and `command`/`args`/`env` for stdio.
 * Skills live separately at ~/.gemini/antigravity-cli/skills/ (handled by sync.js).
 */
const fs = require('fs');
const path = require('path');

function target(home) { return path.join(home, '.gemini', 'config', 'mcp_config.json'); }
function installed(home) { return fs.existsSync(path.join(home, '.gemini')); }

/** @returns {{updated:string[], target:string}} */
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
    cfg.mcpServers[s.name] = s.transport === 'http'
      ? Object.assign({ serverUrl: s.url }, Object.keys(s.headers || {}).length ? { headers: s.headers } : {})
      : { command: s.command, args: s.args || [], env: s.env || {} };
    updated.push(s.name);
  }
  if (!opts.dry) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  }
  return { updated, target: file };
}

module.exports = { id: 'antigravity', installed, projectMcp };
