'use strict';
/*
 * parse.js — dependency-free parsers for the controlled shapes used by this repo.
 * Not a general YAML parser; handles exactly registry.yaml / capabilities.md.
 */
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

// Run a resolver CLI without a shell (execFile — no injection from the ref).
// Returns trimmed stdout, or null if the binary is missing / exits non-zero.
function runCli(bin, args) {
  try {
    return execFileSync(bin, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch (_) {
    return null;
  }
}

/**
 * Pluggable secret backends (ADR 0002 Decision 7). registry only stores refs;
 * the real value is fetched here at sync/render time. Each returns the value or
 * { unresolved } so callers keep the ref and warn (never bakes a raw secret).
 *   op://vault/item/field        → 1Password CLI:  op read <ref>
 *   vault:<path>#<field>         → HashiCorp Vault: vault kv get -field=<field> <path>
 *   keychain:<service>[/<acct>]  → macOS Keychain:  security find-generic-password …
 */
const BACKENDS = {
  op(ref) {
    const v = runCli('op', ['read', ref]);
    return v != null ? v : { unresolved: ref };
  },
  vault(ref) {
    const body = ref.replace(/^vault:/, '');
    const hash = body.lastIndexOf('#');
    if (hash < 0) return { unresolved: ref }; // need a #field selector
    const secretPath = body.slice(0, hash);
    const field = body.slice(hash + 1);
    if (!secretPath || !field) return { unresolved: ref };
    const v = runCli('vault', ['kv', 'get', '-field=' + field, secretPath]);
    return v != null ? v : { unresolved: ref };
  },
  keychain(ref) {
    const body = ref.replace(/^keychain:/, '');
    const slash = body.indexOf('/');
    const service = slash < 0 ? body : body.slice(0, slash);
    const account = slash < 0 ? null : body.slice(slash + 1);
    if (!service) return { unresolved: ref };
    const args = ['find-generic-password', '-s', service, '-w'];
    if (account) args.splice(3, 0, '-a', account);
    const v = runCli('security', args);
    return v != null ? v : { unresolved: ref };
  },
};

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
 * Returns { skills:[{name,source}], mcp:[{name, tools:[]}], hooks:[{name, effect}] }.
 * hooks carry an allow/deny `effect` (ADR 0005): default all-off; a listed hook
 * enables only when effect=allow. There is no `ask` — a hook auto-executes and
 * bypasses the runtime authorization gate, so authorization is this allow/deny.
 */
function parseEnable(file) {
  if (!fs.existsSync(file)) return { skills: [], mcp: [], hooks: [] };
  const body = fs.readFileSync(file, 'utf8').replace(/^---\n[\s\S]*?\n---/, '');
  const out = { skills: [], mcp: [], hooks: [] };
  let top = null; // 'skills' | 'mcp' | 'hooks'
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
    } else if (top === 'hooks') {
      // inline `- { name: x, effect: allow }` or block `- name: x` + `effect: allow|deny`
      const inl = raw.match(/^\s*-\s*\{?\s*name:\s*([A-Za-z0-9_-]+)(?:.*effect:\s*(allow|deny))?/);
      if (inl) { cur = { name: inl[1], effect: inl[2] || 'allow' }; out.hooks.push(cur); continue; }
      const eff = raw.match(/^\s*effect:\s*(allow|deny)\b/);
      if (eff && cur) cur.effect = eff[1];
    }
  }
  return out;
}

/**
 * Parse hooks/registry.yaml -> [{name, event, matcher, command, description,
 * source, 'pinned-ref', checksum}]. Flat shape (2-space indent items, 4-space
 * props), plus a nested `capability:` map for the description. `event` is a
 * canonical event name (see CANONICAL_HOOK_EVENTS in lint.js / projectors).
 */
function parseHookRegistry(text) {
  const hooks = [];
  let cur = null;
  let section = null; // 'capability' | null
  for (const raw of text.split('\n')) {
    const line = raw.replace(/\s+$/, '');
    if (!line.trim() || line.trim().startsWith('#')) continue;
    const indent = line.match(/^\s*/)[0].length;
    const t = line.trim();
    const item = t.match(/^-\s*name:\s*(.+)$/);
    if (item && indent <= 2) { cur = { name: strip(item[1]) }; hooks.push(cur); section = null; continue; }
    if (!cur) continue;
    const opener = t.match(/^([\w-]+):\s*$/);
    if (opener && indent === 4) { section = opener[1]; continue; }
    const kv = t.match(/^([\w-]+):\s*(.*)$/);
    if (!kv) continue;
    if (section === 'capability' && indent >= 6) { if (kv[1] === 'description') cur.description = strip(kv[2]); continue; }
    section = null;
    cur[kv[1]] = strip(kv[2]);
  }
  return hooks;
}

/**
 * Parse bin/registry.yaml -> [{name, source, 'pinned-version', provenance, bin,
 * archive, url, platforms:{ '<os-arch>': {asset, sha256} }}] (ADR 0006, 第四軸).
 * Shape: `tools:` then `  - name: X` items with 4-space props, plus a nested
 * `platforms:` map whose keys (`<os>-<arch>` at indent 6) each hold `asset` /
 * `sha256` at indent 8. Declaration only — the installer fetches + verifies.
 */
function parseBinRegistry(text) {
  const tools = [];
  let cur = null;
  let inPlatforms = false;
  let plat = null;
  for (const raw of text.split('\n')) {
    const line = raw.replace(/\s+$/, '');
    if (!line.trim() || line.trim().startsWith('#')) continue;
    const indent = line.match(/^\s*/)[0].length;
    const t = line.trim();
    const item = t.match(/^-\s*name:\s*(.+)$/);
    if (item && indent <= 2) { cur = { name: strip(item[1]), platforms: {} }; tools.push(cur); inPlatforms = false; plat = null; continue; }
    if (!cur) continue;
    if (indent === 4 && /^platforms:\s*$/.test(t)) { inPlatforms = true; plat = null; continue; }
    if (inPlatforms && indent === 6) {
      const p = t.match(/^([\w-]+):\s*$/);
      if (p) { plat = p[1]; cur.platforms[plat] = {}; continue; }
    }
    if (inPlatforms && indent >= 8 && plat) {
      const kv = t.match(/^([\w-]+):\s*(.*)$/);
      if (kv) cur.platforms[plat][kv[1]] = strip(kv[2]);
      continue;
    }
    if (indent <= 4) {
      inPlatforms = false; plat = null;
      const kv = t.match(/^([\w-]+):\s*(.*)$/);
      if (kv && kv[1] !== 'platforms') cur[kv[1]] = strip(kv[2]);
    }
  }
  return tools;
}

/** extract the leading `---` frontmatter block of a SKILL.md as raw text ('' if none). */
function frontmatterBlock(text) {
  const m = text.match(/^---\n([\s\S]*?)\n---/);
  return m ? m[1] : '';
}

/**
 * Parse a SKILL.md frontmatter's `requires:` -> [{name, min}] (block or inline
 * form). Shared by lint.js (floor/resolve check) and the bin installer (ADR 0006).
 * The `requires:` line may carry a trailing comment.
 */
function parseRequires(fm) {
  const out = [];
  const lines = String(fm).split('\n');
  let i = lines.findIndex((l) => /^requires:\s*(#.*)?$/.test(l));
  if (i < 0) {
    const inl = String(fm).match(/^requires:\s*\[(.+)\]\s*$/m);
    if (inl) {
      const re = /name:\s*([A-Za-z0-9_-]+)(?:[^}]*?min:\s*["']?([0-9][\w.+-]*)["']?)?/g;
      let m; while ((m = re.exec(inl[1]))) out.push({ name: m[1], min: m[2] || null });
    }
    return out;
  }
  for (i = i + 1; i < lines.length; i++) {
    const nm = lines[i].match(/^\s*-\s*name:\s*([A-Za-z0-9_-]+)/);
    if (nm) { out.push({ name: nm[1], min: null }); continue; }
    const mn = lines[i].match(/^\s*min:\s*["']?([0-9][\w.+-]*)["']?/);
    if (mn && out.length) { out[out.length - 1].min = mn[1]; continue; }
    if (/^\S/.test(lines[i])) break; // dedent to next top-level key
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
 * Resolve a secret reference. env: and ${VAR} resolve from dotenv/process.env;
 * op:// / vault: / keychain: go through their pluggable BACKENDS. Anything that
 * can't be resolved (missing CLI, bad ref, non-zero exit) returns {unresolved}
 * so the caller keeps the ref and warns — a raw secret is never baked in.
 */
function resolveRef(val, env) {
  if (typeof val !== 'string') return val;
  if (/^op:\/\//.test(val)) return BACKENDS.op(val);
  if (/^vault:/.test(val)) return BACKENDS.vault(val);
  if (/^keychain:/.test(val)) return BACKENDS.keychain(val);
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

module.exports = { strip, parseRegistry, parseHookRegistry, parseBinRegistry, parseEnable, frontmatterBlock, parseRequires, loadDotenv, resolveRef, resolveServer };
