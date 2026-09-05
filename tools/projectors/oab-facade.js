'use strict';
/*
 * oab-facade projector — register `route: facade` servers as SOURCES behind the OAB
 * MCP Facade, in openab's ~/.openab/agent/mcp.json. The agent's runtime only ever
 * connects to the facade (loopback, no key); the facade holds upstream creds and
 * applies least-privilege. Secrets are written as ${env:VAR} interpolation refs
 * (openab resolves from its process env) — NEVER resolved into the file.
 * SAFE read-modify-write: lossless JSON round-trip, .bak backup, only our keys by name.
 */
const fs = require('fs');
const path = require('path');

function target(home) { return path.join(home, '.openab', 'agent', 'mcp.json'); }
function installed(home) { return fs.existsSync(path.join(home, '.openab')) || true; } // openab pod; harmless elsewhere

// env:VAR -> ${env:VAR} (openab interpolation); literals pass through.
const toRef = (v) => (typeof v === 'string' && /^env:(.+)$/.test(v) ? '${env:' + v.slice(4) + '}' : v);
function mapVals(obj) {
  const out = {};
  for (const [k, v] of Object.entries(obj || {})) out[k] = toRef(v);
  return out;
}

// Build the openab mcp.json entry for a raw registry server (secrets → ${env:} refs).
// Shared by the on-pod projector and the off-pod --render path so both agree.
function shapeServer(s) {
  return s.transport === 'http'
    ? Object.assign({ type: 'http', url: s.url }, Object.keys(s.headers || {}).length ? { headers: mapVals(s.headers) } : {})
    : { type: 'stdio', command: s.command, args: s.args || [], env: mapVals(s.env) };
}

/** @param {Array} servers RAW registry defs (unresolved) @returns {{updated:string[], target:string}} */
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
    cfg.mcpServers[s.name] = shapeServer(s);
    updated.push(s.name);
  }
  if (!opts.dry) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
  }
  return { updated, target: file };
}

module.exports = { id: 'oab-facade', installed, projectMcp, shapeServer };
