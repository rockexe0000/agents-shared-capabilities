'use strict';
/*
 * opencode projector — merge enabled MCP servers into ~/.config/opencode/opencode.json
 * under the `mcp` key. SAFE read-modify-write: JSON round-trip (lossless), .bak backup,
 * only add/update our server keys by name (never rewrites unrelated keys, never auto-removes).
 *
 * opencode MCP shape (https://opencode.ai/docs/mcp-servers):
 *   remote (http/sse): { type: "remote", url, enabled: true, headers? }
 *   local  (stdio):    { type: "local", command: [cmd, ...args], enabled: true, environment? }
 */
const fs = require('fs');
const path = require('path');

function dir(home) { return path.join(home, '.config', 'opencode'); }
function target(home) { return path.join(dir(home), 'opencode.json'); }
function installed(home) { return fs.existsSync(dir(home)) || fs.existsSync(target(home)); }

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
  } else {
    cfg.$schema = 'https://opencode.ai/config.json';
  }
  cfg.mcp = cfg.mcp || {};
  const updated = [];
  for (const s of servers) {
    const entry = s.transport === 'http' || s.transport === 'sse'
      ? Object.assign({ type: 'remote', url: s.url, enabled: true },
          Object.keys(s.headers || {}).length ? { headers: s.headers } : {})
      : Object.assign({ type: 'local', command: [s.command, ...(s.args || [])], enabled: true },
          Object.keys(s.env || {}).length ? { environment: s.env } : {});
    cfg.mcp[s.name] = entry;
    updated.push(s.name);
  }
  if (!opts.dry) {
    fs.mkdirSync(dir(home), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  }
  return { updated, target: file };
}

module.exports = { id: 'opencode', installed, projectMcp };
