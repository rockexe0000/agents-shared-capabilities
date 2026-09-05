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

const REPO = path.resolve(__dirname, '..');
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

if (errors.length) {
  console.error('lint FAIL:');
  for (const e of errors) console.error('  ERROR ' + e);
  process.exit(1);
}
console.log('lint OK — capability catalog valid');
