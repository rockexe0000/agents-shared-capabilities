#!/usr/bin/env node
'use strict';
/*
 * sync.js — project the enabled subset of the catalog into each installed runtime.
 *
 *   閘 1(存在):讀 ~/personal/capabilities.md enable-list;預設全關。
 *   Skills → 逐一 symlink 進各 runtime skills 目錄(個人覆蓋 catalog;撞名報錯)。
 *   MCP    → 解析 registry + 解密 secret 參照 → 各 runtime projector 安全 merge。
 *
 * 授權(閘 2)不在此:skill script / MCP tool 執行時過 Hot Permission Boundary。
 * Usage: node tools/sync.js [--dry-run]
 */
const fs = require('fs');
const path = require('path');
const os = require('os');
const { parseRegistry, parseEnable, loadDotenv, resolveServer } = require('./lib/parse');

const REPO = path.resolve(__dirname, '..');
const HOME = process.env.HOME || os.homedir();
const DRY = process.argv.includes('--dry-run');

const SKILL_RUNTIMES = [
  { id: 'claude-code', skillsDir: path.join(HOME, '.claude', 'skills') },
  { id: 'codex', skillsDir: path.join(HOME, '.codex', 'skills') },
];
const MCP_PROJECTORS = [require('./projectors/claude-code'), require('./projectors/codex')];

function skillSource(name, source) {
  if (source === 'catalog') return path.join(REPO, 'skills', name);
  return null; // personal namespace (agent-bot/{uid}/skills) — TODO resolve
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

const enable = parseEnable(path.join(HOME, 'personal', 'capabilities.md'));

// ---- skills ----
console.log(`enabled skills: ${enable.skills.map((s) => s.name).join(', ') || '(none)'}`);
for (const rt of SKILL_RUNTIMES) {
  console.log(`\n[${rt.id} skills]`);
  for (const s of enable.skills) {
    const src = skillSource(s.name, s.source);
    if (!src) { console.log(`  ${s.name}: TODO resolve personal namespace (agent-bot/{uid}/skills)`); continue; }
    if (!fs.existsSync(src)) { console.log(`  ${s.name}: ERROR source missing (${src})`); continue; }
    console.log(`  ${s.name}: ${linkSkill(rt, s.name, src)}`);
  }
}

// ---- MCP ----
console.log(`\nenabled MCP servers: ${enable.mcp.map((s) => s.name).join(', ') || '(none)'}`);
if (enable.mcp.length) {
  const regFile = path.join(REPO, 'mcp', 'registry.yaml');
  const registry = fs.existsSync(regFile) ? parseRegistry(fs.readFileSync(regFile, 'utf8')) : [];
  const byName = new Map(registry.map((s) => [s.name, s]));
  const env = loadDotenv(REPO);
  const resolved = [];
  for (const want of enable.mcp) {
    const def = byName.get(want.name);
    if (!def) { console.log(`  ${want.name}: ERROR not in registry`); continue; }
    const { server, unresolved } = resolveServer(def, env);
    if (unresolved.length) console.log(`  ${want.name}: WARN unresolved secret(s): ${unresolved.join(', ')}`);
    resolved.push(server);
  }
  for (const p of MCP_PROJECTORS) {
    if (!p.installed(HOME)) { console.log(`\n[${p.id} mcp] skip (not installed)`); continue; }
    const r = p.projectMcp(resolved, HOME, { dry: DRY });
    console.log(`\n[${p.id} mcp] ${DRY ? 'would update' : 'updated'} ${r.updated.join(', ') || '(none)'} → ${r.target}${DRY ? '' : ' (.bak saved)'}`);
  }
}

console.log('\ndone' + (DRY ? ' (dry-run)' : ''));
