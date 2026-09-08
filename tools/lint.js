#!/usr/bin/env node
'use strict';
/*
 * lint.js — validate the capability catalog.
 *   - each skills/<name>/SKILL.md: frontmatter name/description sanity
 *   - mcp/registry.yaml: parseable, server names unique, secrets are references (not raw values)
 * Dependency-free; tailored to the controlled shapes documented in templates/.
 *
 * Usage: node tools/lint.js
 */
const fs = require('fs');
const path = require('path');
const { parseHookRegistry, parseBinRegistry } = require('./lib/parse');

const REPO = path.resolve(__dirname, '..');
const CANONICAL_HOOK_EVENTS = ['pre-tool', 'post-tool', 'session-start', 'stop', 'user-prompt-submit'];
const errors = [];
const err = (m) => errors.push(m);

/** extract the leading `---` frontmatter block as raw text (or null) */
function frontmatter(text) {
  const m = text.match(/^---\n([\s\S]*?)\n---/);
  return m ? m[1] : null;
}
/** shallow scalar lookup `key:` at top level of a frontmatter block */
function scalar(fm, key) {
  const re = new RegExp('^' + key + ':\\s*(.+)$', 'm');
  const m = fm.match(re);
  if (!m) return null;
  return m[1].trim().replace(/^["']|["']$/g, '');
}

/** semver-ish compare (strip leading v/=, numeric per dotted field). a<b:-1, a==b:0, a>b:1. */
function cmpVer(a, b) {
  const norm = (v) => String(v).replace(/^[v=]/, '').split(/[.+-]/).map((n) => parseInt(n, 10));
  const A = norm(a), B = norm(b);
  for (let i = 0; i < Math.max(A.length, B.length); i++) {
    const x = A[i] || 0, y = B[i] || 0;
    if (isNaN(x) || isNaN(y)) return 0; // non-numeric field → don't gate
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

/** extract a SKILL.md `requires:` frontmatter block -> [{name, min}] (block or inline form). */
function parseRequires(fm) {
  const out = [];
  const lines = fm.split('\n');
  let i = lines.findIndex((l) => /^requires:\s*(#.*)?$/.test(l));
  if (i < 0) {
    const inl = fm.match(/^requires:\s*\[(.+)\]\s*$/m);
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

// ---- bin registry (ADR 0006, 第四軸) ----
// External binary/CLI deps a skill shells out to. External source ⇒ must be
// pinned + per-platform checksummed (mirrors the hooks supply-chain rule).
const binTools = new Map(); // name -> parsed tool entry (for skill `requires` cross-check)
const binReg = path.join(REPO, 'bin', 'registry.yaml');
if (fs.existsSync(binReg)) {
  const tools = parseBinRegistry(fs.readFileSync(binReg, 'utf8'));
  const names = new Set();
  for (const t of tools) {
    const at = `bin/registry.yaml '${t.name || '(unnamed)'}'`;
    if (!t.name) { err(`${at}: missing name`); continue; }
    if (!/^[a-z0-9-]{1,64}$/.test(t.name)) err(`${at}: name must be kebab-case, ≤64`);
    if (names.has(t.name)) err(`${at}: duplicate tool name`);
    names.add(t.name);
    binTools.set(t.name, t);
    if (!t.source) err(`${at}: missing source`);
    if (t.provenance && !['none', 'attestation', 'cosign'].includes(t.provenance)) {
      err(`${at}: provenance must be none|attestation|cosign`);
    }
    const external = /^external:/i.test(t.source || '');
    const plats = Object.entries(t.platforms || {});
    if (external) {
      if (!t['pinned-version'] || /^n\/a$/i.test(t['pinned-version'])) err(`${at}: external tool needs a real pinned-version`);
      if (!plats.length) err(`${at}: external tool needs at least one platform`);
    }
    for (const [p, spec] of plats) {
      if (!spec.asset) err(`${at}: platform '${p}' missing asset`);
      if (external || spec.sha256) {
        if (!/^[a-f0-9]{64}$/i.test(spec.sha256 || '')) err(`${at}: platform '${p}' sha256 must be 64 hex`);
      }
    }
  }
}

// ---- skills ----
const skillsDir = path.join(REPO, 'skills');
if (fs.existsSync(skillsDir)) {
  for (const name of fs.readdirSync(skillsDir)) {
    const dir = path.join(skillsDir, name);
    if (!fs.statSync(dir).isDirectory()) continue;
    const sk = path.join(dir, 'SKILL.md');
    if (!fs.existsSync(sk)) { err(`skills/${name}: missing SKILL.md`); continue; }
    const fm = frontmatter(fs.readFileSync(sk, 'utf8'));
    if (!fm) { err(`skills/${name}/SKILL.md: missing frontmatter block`); continue; }
    const nm = scalar(fm, 'name');
    const desc = scalar(fm, 'description');
    if (!nm) err(`skills/${name}/SKILL.md: frontmatter 'name' missing`);
    else {
      if (nm !== name) err(`skills/${name}/SKILL.md: name '${nm}' != folder '${name}'`);
      if (!/^[a-z0-9-]{1,64}$/.test(nm)) err(`skills/${name}: name must be kebab-case, ≤64`);
      if (/claude|anthropic/i.test(nm)) err(`skills/${name}: name must not contain claude/anthropic`);
    }
    if (!desc) err(`skills/${name}/SKILL.md: frontmatter 'description' missing/empty`);
    // requires: each declared binary must resolve to bin/registry.yaml, and the
    // registry's pinned-version must satisfy the skill's floor (ADR 0006 D2).
    for (const r of parseRequires(fm)) {
      const tool = binTools.get(r.name);
      if (!tool) { err(`skills/${name}: requires '${r.name}' has no bin/registry.yaml entry`); continue; }
      const pinned = tool['pinned-version'];
      if (r.min && pinned && !/^n\/a$/i.test(pinned) && cmpVer(pinned, r.min) < 0) {
        err(`skills/${name}: requires ${r.name} ≥ ${r.min} but bin registry pins ${pinned}`);
      }
    }
  }
}

// ---- mcp registry ----
const reg = path.join(REPO, 'mcp', 'registry.yaml');
if (fs.existsSync(reg)) {
  const lines = fs.readFileSync(reg, 'utf8').split('\n');
  const names = new Set();
  lines.forEach((ln, i) => {
    const nm = ln.match(/^\s*-\s*name:\s*(.+)$/);
    if (nm) {
      const v = nm[1].trim();
      if (names.has(v)) err(`mcp/registry.yaml:${i + 1}: duplicate server name '${v}'`);
      names.add(v);
    }
    // secret hygiene: an env/header value that looks like a raw secret, not a reference
    const ev = ln.match(/^\s{6,}[A-Za-z0-9_-]+:\s*["']?([^"'#]+)/);
    if (ev) {
      const val = ev[1].trim();
      const isRef = /^(env:|op:\/\/|vault:|\$\{)/.test(val);
      if (!isRef && /(token|secret|key|password|authorization)/i.test(ln)) {
        err(`mcp/registry.yaml:${i + 1}: value looks like a raw secret; use a reference (env:/op://vault:)`);
      }
    }
  });
}

// ---- hooks registry (ADR 0005) ----
const hookReg = path.join(REPO, 'hooks', 'registry.yaml');
if (fs.existsSync(hookReg)) {
  const hooks = parseHookRegistry(fs.readFileSync(hookReg, 'utf8'));
  const names = new Set();
  for (const h of hooks) {
    const at = `hooks/registry.yaml '${h.name || '(unnamed)'}'`;
    if (!h.name) { err(`${at}: missing name`); continue; }
    if (!/^[a-z0-9-]{1,64}$/.test(h.name)) err(`${at}: name must be kebab-case, ≤64`);
    if (names.has(h.name)) err(`${at}: duplicate hook name`);
    names.add(h.name);
    if (!h.event) err(`${at}: missing event`);
    else if (!CANONICAL_HOOK_EVENTS.includes(h.event)) {
      err(`${at}: event '${h.event}' not canonical (${CANONICAL_HOOK_EVENTS.join(' | ')})`);
    }
    if (!h.command) err(`${at}: missing command`);
    // external hook = auto-exec third-party code → must be pinned + checksummed (ADR 0002 D6 / 0005 D4)
    if (/^external:/i.test(h.source || '')) {
      if (!h['pinned-ref'] || /^n\/a$/i.test(h['pinned-ref'])) err(`${at}: external hook needs a real pinned-ref`);
      if (!h.checksum || /^n\/a$/i.test(h.checksum)) err(`${at}: external hook needs a checksum`);
    }
  }
}

if (errors.length) {
  console.error('lint FAIL:');
  for (const e of errors) console.error('  ERROR ' + e);
  process.exit(1);
}
console.log('lint OK — capability catalog valid');
