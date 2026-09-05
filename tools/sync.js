#!/usr/bin/env node
'use strict';
/*
 * sync.js — project the enabled subset of the catalog into each installed runtime.
 *
 *   閘 1(存在):讀 ~/personal/capabilities.md 的 enable-list;預設全關。
 *   Skills → 逐一 symlink 進各 runtime skills 目錄(個人覆蓋 catalog;撞名報錯)。
 *   MCP    → 產生各 runtime 的 config snippet 到 .sync-output/(不 clobber live config)。
 *
 * 授權(閘 2)不在此:skill script / MCP tool 執行時過 Hot Permission Boundary。
 * Usage: node tools/sync.js [--dry-run]
 */
const fs = require('fs');
const path = require('path');

const REPO = path.resolve(__dirname, '..');
const HOME = process.env.HOME || require('os').homedir();
const DRY = process.argv.includes('--dry-run');

const RUNTIMES = [
  { id: 'claude-code', skillsDir: path.join(HOME, '.claude', 'skills') },
  { id: 'codex', skillsDir: path.join(HOME, '.codex', 'skills') },
];

/** parse the simple `skills:` list from ~/personal/capabilities.md (`- name: X` / `source: Y`) */
function readEnable() {
  const f = path.join(HOME, 'personal', 'capabilities.md');
  if (!fs.existsSync(f)) { console.error(`no ${f} — nothing enabled (default all-off)`); return { skills: [], mcp: [] }; }
  const body = fs.readFileSync(f, 'utf8').replace(/^---\n[\s\S]*?\n---/, '');
  const skills = [];
  let inSkills = false;
  let cur = null;
  for (const raw of body.split('\n')) {
    const topKey = raw.match(/^(\w[\w-]*):\s*$/);
    if (topKey) { inSkills = topKey[1] === 'skills'; continue; }
    if (!inSkills) continue;
    const nm = raw.match(/^\s*-\s*name:\s*(\S[^\n]*?)\s*$/);
    if (nm) { cur = { name: nm[1], source: 'catalog' }; skills.push(cur); continue; }
    const src = raw.match(/^\s*source:\s*(\S+)/);
    if (src && cur) cur.source = src[1];
  }
  return { skills, mcp: [] };
}

function skillSource(name, source) {
  if (source === 'catalog') return path.join(REPO, 'skills', name);
  // personal → agent's cold namespace; uid not known here → documented TODO
  return null;
}

function linkSkill(rt, name, src) {
  const dest = path.join(rt.skillsDir, name);
  if (!fs.existsSync(rt.skillsDir)) return `skip (${rt.id} not installed)`;
  if (fs.existsSync(dest)) {
    const real = fs.lstatSync(dest).isSymbolicLink() ? fs.readlinkSync(dest) : null;
    if (real === src) return 'up-to-date';
    if (real === null) return `CONFLICT: ${dest} exists and is not our symlink — skipped`;
  }
  if (DRY) return `would link → ${src}`;
  try { fs.rmSync(dest, { force: true }); } catch (_) {}
  fs.symlinkSync(src, dest);
  return `linked → ${src}`;
}

const enable = readEnable();
console.log(`enabled skills: ${enable.skills.map((s) => s.name).join(', ') || '(none)'}`);

for (const rt of RUNTIMES) {
  console.log(`\n[${rt.id}]`);
  for (const s of enable.skills) {
    const src = skillSource(s.name, s.source);
    if (!src) { console.log(`  ${s.name}: TODO resolve personal namespace (agent-bot/{uid}/skills)`); continue; }
    if (!fs.existsSync(src)) { console.log(`  ${s.name}: ERROR source missing (${src})`); continue; }
    console.log(`  ${s.name}: ${linkSkill(rt, s.name, src)}`);
  }
}

// MCP projection — safe MVP: emit intended config, do not touch live runtime config.
console.log('\n[mcp] projection: TODO — emit per-runtime snippets to .sync-output/ (see projectors/).');
console.log('done' + (DRY ? ' (dry-run)' : ''));
