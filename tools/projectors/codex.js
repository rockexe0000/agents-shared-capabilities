'use strict';
/*
 * codex projector — write enabled MCP servers into ~/.codex/config.toml as a single
 * managed block delimited by markers. SAFE: preserves all non-managed content;
 * regenerates only the block between the markers; .bak backup.
 */
const fs = require('fs');
const path = require('path');

const BEGIN = '# >>> agents-shared-capabilities (managed) — do not edit by hand';
const END = '# <<< agents-shared-capabilities';

function target(home) { return path.join(home, '.codex', 'config.toml'); }
function installed(home) { return fs.existsSync(path.join(home, '.codex')); }

const q = (v) => '"' + String(v).replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';

function toToml(servers) {
  const out = [BEGIN];
  for (const s of servers) {
    out.push(`[mcp_servers.${s.name}]`);
    if (s.transport === 'http') {
      out.push(`url = ${q(s.url)}`);
    } else {
      out.push(`command = ${q(s.command)}`);
      out.push(`args = [${(s.args || []).map(q).join(', ')}]`);
    }
    const env = Object.entries(s.env || {}).filter(([, v]) => v !== '');
    if (env.length) {
      out.push(`[mcp_servers.${s.name}.env]`);
      for (const [k, v] of env) out.push(`${k} = ${q(v)}`);
    }
    out.push('');
  }
  out.push(END);
  return out.join('\n');
}

/** @returns {{updated:string[], target:string}} */
function projectMcp(servers, home, opts = {}) {
  const file = target(home);
  let text = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
  if (fs.existsSync(file) && !opts.dry) fs.copyFileSync(file, file + '.bak');
  // strip any existing managed block
  const re = new RegExp('\\n*' + BEGIN.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '[\\s\\S]*?' + END.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g');
  text = text.replace(re, '').replace(/\s+$/, '');
  const block = toToml(servers);
  const next = (text ? text + '\n\n' : '') + block + '\n';
  if (!opts.dry) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, next);
  }
  return { updated: servers.map((s) => s.name), target: file };
}

module.exports = { id: 'codex', installed, projectMcp };
