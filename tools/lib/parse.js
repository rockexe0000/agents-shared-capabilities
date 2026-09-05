'use strict';
/*
 * parse.js — dependency-free parsers for the controlled shapes used by this repo.
 * Not a general YAML parser; handles exactly registry.yaml / capabilities.md.
 */
const fs = require('fs');
const path = require('path');

// Drop a trailing ` # comment`, but not a `#` inside a quoted scalar.
function stripComment(s) {
  let q = null;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (q) { if (c === q) q = null; continue; }
    if (c === '"' || c === "'") { q = c; continue; }
    if (c === '#' && (i === 0 || s[i - 1] === ' ' || s[i - 1] === '\t')) return s.slice(0, i);
  }
  return s;
}
const strip = (s) => (s == null ? s : stripComment(String(s)).trim().replace(/^["']|["']$/g, ''));

/**
 * Parse mcp/registry.yaml -> [{name, transport, command, args, url, auth, env:{}}]
 * Structure (2-space indent): `servers:` then `  - name: X` items with 4-space
 * props; `args:` is an inline JSON array; `env:`/`capability:` are nested maps.
 */
function parseRegistry(text) {
  const servers = [];
  let cur = null;
  let section = null; // 'env' | 'capability' | null
  for (const raw of text.split('\n')) {
    const line = raw.replace(/\s+$/, '');
    if (!line.trim() || line.trim().startsWith('#')) continue;
    const indent = line.match(/^\s*/)[0].length;
    const t = line.trim();
    const item = t.match(/^-\s*name:\s*(.+)$/);
    if (item && indent <= 2) { cur = { name: strip(item[1]), env: {}, headers: {} }; servers.push(cur); section = null; continue; }
    if (!cur) continue;
    const opener = t.match(/^([\w-]+):\s*$/);
    if (opener && indent === 4) { section = opener[1]; continue; }
    const kv = t.match(/^([\w-]+):\s*(.*)$/);
    if (!kv) continue;
    if (section && indent >= 6) {
      if (section === 'env') cur.env[kv[1]] = strip(kv[2]);
      else if (section === 'headers') cur.headers[kv[1]] = strip(kv[2]);
      continue;
    }
    section = null;
    if (kv[1] === 'args') { try { cur.args = JSON.parse(stripComment(kv[2])); } catch (_) { cur.args = []; } }
    else cur[kv[1]] = strip(kv[2]);
  }
  return servers;
}

/**
 * Parse ~/personal/capabilities.md enable-list.
 * Returns { skills:[{name,source}], mcp:[{name, tools:[]}] }.
 */
function parseEnable(file) {
  if (!fs.existsSync(file)) return { skills: [], mcp: [] };
  const body = fs.readFileSync(file, 'utf8').replace(/^---\n[\s\S]*?\n---/, '');
  const out = { skills: [], mcp: [] };
  let top = null; // 'skills' | 'mcp'
  let mcpServers = false;
  let cur = null;
  for (const raw of body.split('\n')) {
    if (!raw.trim() || raw.trim().startsWith('#')) continue;
    const topKey = raw.match(/^(\w[\w-]*):\s*$/);
    if (topKey) { top = topKey[1]; mcpServers = false; cur = null; continue; }
    if (top === 'skills') {
      const nm = raw.match(/^\s*-\s*name:\s*(\S[^\n]*?)\s*$/);
      if (nm) { cur = { name: nm[1], source: 'catalog' }; out.skills.push(cur); continue; }
      const src = raw.match(/^\s*source:\s*(\S+)/);
      if (src && cur) cur.source = src[1];
    } else if (top === 'mcp') {
      if (/^\s*servers:\s*$/.test(raw)) { mcpServers = true; continue; }
      // inline form: `- { name: x, tools: [...] }` or block `- name: x`
      const inl = raw.match(/^\s*-\s*\{?\s*name:\s*([A-Za-z0-9_-]+)/);
      if (mcpServers && inl) { cur = { name: inl[1], tools: [] }; out.mcp.push(cur); }
    }
  }
  return out;
}

/** load secrets/.env into a plain object (KEY=VALUE) */
function loadDotenv(repo) {
  const f = path.join(repo, 'secrets', '.env');
  const out = {};
  if (!fs.existsSync(f)) return out;
  for (const l of fs.readFileSync(f, 'utf8').split('\n')) {
    const m = l.match(/^([A-Za-z0-9_]+)=(.*)$/);
    if (m && !l.trim().startsWith('#')) out[m[1]] = m[2];
  }
  return out;
}

/**
 * Resolve a secret reference. env: and ${VAR} resolve from dotenv/process.env.
 * op:// and vault: are left unresolved (resolver not yet implemented) -> {unresolved}.
 */
function resolveRef(val, env) {
  if (typeof val !== 'string') return val;
  if (/^(op:\/\/|vault:)/.test(val)) return { unresolved: val };
  const m = val.match(/^env:(.+)$/);
  if (m) return env[m[1]] != null ? env[m[1]] : (process.env[m[1]] != null ? process.env[m[1]] : { unresolved: val });
  return val.replace(/\$\{(\w+)\}/g, (_, v) => (env[v] != null ? env[v] : (process.env[v] != null ? process.env[v] : '')));
}

/**
 * Resolve a server's env, args, url, and headers. Returns {server, unresolved:[]}.
 * headers are kept BOTH resolved (out.headers, for value-in projectors like
 * claude-code/antigravity) and as env-var names (out.headerEnv, for codex
 * env_http_headers which references the env var by name, keeping the secret out of file).
 */
function resolveServer(s, env) {
  const unresolved = [];
  const out = Object.assign({}, s);
  out.env = {};
  for (const [k, v] of Object.entries(s.env || {})) {
    const r = resolveRef(v, env);
    if (r && r.unresolved) { unresolved.push(`${s.name}.env.${k}=${r.unresolved}`); out.env[k] = ''; }
    else out.env[k] = r;
  }
  out.args = (s.args || []).map((a) => {
    const r = resolveRef(a, env);
    return r && r.unresolved ? a : r;
  });
  if (s.url) {
    for (const [, v] of [...s.url.matchAll(/\$\{(\w+)\}/g)]) {
      if (env[v] == null && process.env[v] == null) unresolved.push(`${s.name}.url:${v}`);
    }
    out.url = s.url.replace(/\$\{(\w+)\}/g, (_, v) => (env[v] != null ? env[v] : (process.env[v] != null ? process.env[v] : `\${${v}}`)));
  }
  out.headers = {};
  out.headerEnv = {};
  for (const [k, v] of Object.entries(s.headers || {})) {
    const m = typeof v === 'string' && v.match(/^env:(.+)$/);
    if (m) {
      out.headerEnv[k] = m[1];
      const r = resolveRef(v, env);
      if (r && r.unresolved) { unresolved.push(`${s.name}.headers.${k}=${r.unresolved}`); out.headers[k] = ''; }
      else out.headers[k] = r;
    } else {
      out.headers[k] = v;
    }
  }
  return { server: out, unresolved };
}

module.exports = { strip, parseRegistry, parseEnable, loadDotenv, resolveRef, resolveServer };
