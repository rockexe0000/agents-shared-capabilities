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
const { parseRegistry, parseHookRegistry, parseEnable, loadDotenv, resolveServer } = require('./lib/parse');

const REPO = path.resolve(__dirname, '..');
const HOME = process.env.HOME || os.homedir();
const DRY = process.argv.includes('--dry-run');

const SKILL_RUNTIMES = [
  { id: 'claude-code', base: path.join(HOME, '.claude'), skillsDir: path.join(HOME, '.claude', 'skills') },
  { id: 'codex', base: path.join(HOME, '.codex'), skillsDir: path.join(HOME, '.codex', 'skills') },
  { id: 'antigravity', base: path.join(HOME, '.gemini'), skillsDir: path.join(HOME, '.gemini', 'antigravity-cli', 'skills') },
  { id: 'opencode', base: path.join(HOME, '.config', 'opencode'), skillsDir: path.join(HOME, '.config', 'opencode', 'skills') },
];
// direct-route projectors: write the agent-facing runtime MCP config.
const MCP_PROJECTORS = [
  require('./projectors/claude-code'),
  require('./projectors/codex'),
  require('./projectors/antigravity'),
  require('./projectors/opencode'),
];
// facade-route projector: register sources behind the OAB MCP Facade (openab mcp.json).
const FACADE_PROJECTOR = require('./projectors/oab-facade');
// hook projectors (ADR 0005). MVP: claude-code only; others join as they gain projectHooks.
const HOOK_PROJECTORS = [require('./projectors/claude-code')];
const DEFAULT_ROUTE = 'facade'; // policy: MCP hides behind oab-facade unless marked `route: direct`

// --render <outdir> --capabilities <file>: emit config ARTIFACTS off-pod (for infra to
// bake as a configMap) instead of projecting into a live HOME. Secrets stay ${env:} refs.
if (process.argv.includes('--render')) { render(); process.exit(0); }
if (process.argv.includes('--check')) { check(); /* check() exits */ }

// Resolve the agent's cold namespace base (agent-bot/{uid}) from ~/personal, which
// is symlinked to agent-bot/{uid}/personal. Its parent dir holds the sibling
// skills/ and mcp/ personal namespaces (ADR 0002 Decision 3). null if not mounted.
function personalBase() {
  try {
    const real = fs.realpathSync(path.join(HOME, 'personal')); // …/agent-bot/{uid}/personal
    return path.dirname(real);                                 // …/agent-bot/{uid}
  } catch (_) {
    return null;
  }
}

// Load catalog + personal MCP registries into one name→def map; personal overrides
// catalog on name collision (ADR 0002 Decision 3: 個人覆蓋 catalog).
function buildRegistry(base) {
  const load = (f) => (fs.existsSync(f) ? parseRegistry(fs.readFileSync(f, 'utf8')) : []);
  const byName = new Map(load(path.join(REPO, 'mcp', 'registry.yaml')).map((s) => [s.name, s]));
  if (base) for (const s of load(path.join(base, 'mcp', 'registry.yaml'))) byName.set(s.name, s);
  return byName;
}

// Same as buildRegistry but for hooks/registry.yaml (ADR 0005). 個人覆蓋 catalog.
function buildHookRegistry(base) {
  const load = (f) => (fs.existsSync(f) ? parseHookRegistry(fs.readFileSync(f, 'utf8')) : []);
  const byName = new Map(load(path.join(REPO, 'hooks', 'registry.yaml')).map((h) => [h.name, h]));
  if (base) for (const h of load(path.join(base, 'hooks', 'registry.yaml'))) byName.set(h.name, h);
  return byName;
}

function skillSource(name, source, base) {
  if (source === 'catalog') return path.join(REPO, 'skills', name);
  if (source === 'personal') return base ? path.join(base, 'skills', name) : null;
  return null;
}

function linkSkill(rt, name, src) {
  if (!fs.existsSync(rt.base)) return `skip (${rt.id} not installed)`;
  if (!DRY) fs.mkdirSync(rt.skillsDir, { recursive: true });
  const dest = path.join(rt.skillsDir, name);
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
const PBASE = personalBase();
console.log(`personal namespace: ${PBASE || '(unresolved — ~/personal not a symlink into agent-bot/{uid})'}`);

// ---- skills ----
console.log(`enabled skills: ${enable.skills.map((s) => s.name).join(', ') || '(none)'}`);
for (const rt of SKILL_RUNTIMES) {
  console.log(`\n[${rt.id} skills]`);
  for (const s of enable.skills) {
    const src = skillSource(s.name, s.source, PBASE);
    if (!src) { console.log(`  ${s.name}: ERROR cannot resolve source=${s.source} (personal namespace not mounted)`); continue; }
    if (!fs.existsSync(src)) { console.log(`  ${s.name}: ERROR source missing (${src})`); continue; }
    console.log(`  ${s.name}: ${linkSkill(rt, s.name, src)}`);
  }
}

// ---- MCP ----
console.log(`\nenabled MCP servers: ${enable.mcp.map((s) => s.name).join(', ') || '(none)'}`);
if (enable.mcp.length) {
  const byName = buildRegistry(PBASE);
  const env = loadDotenv(REPO);
  const direct = [];   // resolved defs → runtime configs
  const facade = [];   // raw defs → openab mcp.json (openab resolves ${env:} itself)
  for (const want of enable.mcp) {
    const def = byName.get(want.name);
    if (!def) { console.log(`  ${want.name}: ERROR not in registry`); continue; }
    const route = def.route || DEFAULT_ROUTE;
    if (route === 'direct') {
      const { server, unresolved } = resolveServer(def, env);
      if (unresolved.length) console.log(`  ${want.name} (direct): WARN unresolved secret(s): ${unresolved.join(', ')}`);
      direct.push(server);
    } else {
      facade.push(def); // behind oab-facade; secrets stay as ${env:} refs
      console.log(`  ${want.name}: route=facade (behind oab-facade)`);
    }
  }
  for (const p of MCP_PROJECTORS) {
    if (!p.installed(HOME)) { console.log(`\n[${p.id} mcp] skip (not installed)`); continue; }
    const r = p.projectMcp(direct, HOME, { dry: DRY });
    console.log(`\n[${p.id} mcp] ${DRY ? 'would update' : 'updated'} ${r.updated.join(', ') || '(none)'} → ${r.target}${DRY ? '' : ' (.bak saved)'}`);
  }
  if (facade.length) {
    const r = FACADE_PROJECTOR.projectMcp(facade, HOME, { dry: DRY });
    console.log(`\n[oab-facade sources] ${DRY ? 'would register' : 'registered'} ${r.updated.join(', ')} → ${r.target}${DRY ? '' : ' (.bak saved)'}`);
  }
}

// ---- hooks (ADR 0005) ----
// Only effect=allow is projected; deny/unlisted is not (default all-off). We still
// run the projector when the enabled set is empty so a now-disabled hook is stripped
// from configs that still carry a previously-projected entry.
const enabledHooks = enable.hooks.filter((h) => h.effect === 'allow');
const deniedHooks = enable.hooks.filter((h) => h.effect === 'deny');
console.log(`\nenabled hooks (allow): ${enabledHooks.map((h) => h.name).join(', ') || '(none)'}` +
  (deniedHooks.length ? `  |  deny: ${deniedHooks.map((h) => h.name).join(', ')}` : ''));
{
  const byName = buildHookRegistry(PBASE);
  const resolved = [];
  for (const want of enabledHooks) {
    const def = byName.get(want.name);
    if (!def) { console.log(`  ${want.name}: ERROR not in hook registry`); continue; }
    resolved.push(def);
  }
  for (const p of HOOK_PROJECTORS) {
    if (typeof p.projectHooks !== 'function') continue;
    if (!p.installed(HOME)) { console.log(`\n[${p.id} hooks] skip (not installed)`); continue; }
    const r = p.projectHooks(resolved, HOME, { dry: DRY });
    const skip = r.skipped.length ? `; skipped(unmapped): ${r.skipped.map((s) => `${s.name}(${s.event})`).join(', ')}` : '';
    console.log(`\n[${p.id} hooks] ${DRY ? 'would set' : 'set'} ${r.updated.join(', ') || '(none)'} → ${r.target}${DRY ? '' : ' (.bak saved)'}${skip}`);
  }
}

console.log('\ndone' + (DRY ? ' (dry-run)' : ''));

// ---- render / check (off-pod artifact generation; MCP only) ----
function capFileArg() {
  const ci = process.argv.indexOf('--capabilities');
  return ci >= 0 ? process.argv[ci + 1] : path.join(HOME, 'personal', 'capabilities.md');
}

// Build the two rendered artifacts (as pretty JSON text) from a capabilities.md.
// Shared by --render (write) and --check (compare) so both see identical output.
function buildArtifacts(capFile) {
  // personal base from the capabilities file: agent-bot/{uid}/personal/capabilities.md → agent-bot/{uid}
  let base = null;
  try { base = path.dirname(path.dirname(fs.realpathSync(capFile))); } catch (_) {}
  const byName = buildRegistry(base);
  const enable = parseEnable(capFile);
  const facade = [];
  const direct = [];
  for (const want of enable.mcp) {
    const def = byName.get(want.name);
    if (!def) { console.error(`  ${want.name}: not in registry — skipped`); continue; }
    ((def.route || DEFAULT_ROUTE) === 'direct' ? direct : facade).push(def);
  }
  const shape = FACADE_PROJECTOR.shapeServer;
  const asCfg = (list) => JSON.stringify({ mcpServers: Object.fromEntries(list.map((s) => [s.name, shape(s)])) }, null, 2) + '\n';
  return {
    facade, direct,
    files: { 'openab-agent-mcp.json': asCfg(facade), 'runtime-mcp.json': asCfg(direct) },
  };
}

function render() {
  const outDir = process.argv[process.argv.indexOf('--render') + 1];
  if (!outDir || outDir.startsWith('--')) {
    console.error('usage: node sync.js --render <outdir> [--capabilities <capabilities.md>]');
    process.exit(1);
  }
  const capFile = capFileArg();
  const { facade, direct, files } = buildArtifacts(capFile);
  fs.mkdirSync(outDir, { recursive: true });
  for (const [name, text] of Object.entries(files)) fs.writeFileSync(path.join(outDir, name), text);
  console.log(`rendered openab-agent-mcp.json (facade: ${facade.map((s) => s.name).join(', ') || 'none'})`);
  console.log(`rendered runtime-mcp.json    (direct: ${direct.map((s) => s.name).join(', ') || 'none'}) → ${outDir}`);
}

// Drift check: re-render from the catalog + capabilities.md and compare against the
// committed artifacts in <committedDir> (option-C GitOps: infra bakes them as a
// configMap). Exits 1 on drift so CI / a cron can fail loudly. Deterministic, no HOME.
function check() {
  const dir = process.argv[process.argv.indexOf('--check') + 1];
  if (!dir || dir.startsWith('--')) {
    console.error('usage: node sync.js --check <committed-artifacts-dir> [--capabilities <capabilities.md>]');
    process.exit(1);
  }
  const { files } = buildArtifacts(capFileArg());
  let drift = false;
  for (const [name, want] of Object.entries(files)) {
    const committed = path.join(dir, name);
    const have = fs.existsSync(committed) ? fs.readFileSync(committed, 'utf8') : null;
    if (have === want) { console.log(`  ${name}: IN SYNC`); continue; }
    drift = true;
    if (have === null) { console.error(`  ${name}: DRIFT — missing in ${dir}`); continue; }
    console.error(`  ${name}: DRIFT — committed differs from freshly rendered`);
    // minimal line-level hint
    const w = want.split('\n'), h = have.split('\n');
    for (let i = 0; i < Math.max(w.length, h.length); i++) {
      if (w[i] !== h[i]) {
        if (h[i] !== undefined) console.error(`      - committed: ${h[i]}`);
        if (w[i] !== undefined) console.error(`      + rendered:  ${w[i]}`);
      }
    }
  }
  console.log(drift ? '\ndrift detected — re-render and commit the artifacts' : '\nno drift — committed artifacts are current');
  process.exit(drift ? 1 : 0);
}
