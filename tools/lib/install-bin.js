'use strict';
/*
 * install-bin.js — the dev/host backend of the bin/ axis (ADR 0006 Phase B).
 *
 * Given the tools an agent's enabled skills `require`, fetch the pinned release
 * asset for THIS platform, verify its sha256, extract the binary into an on-PATH
 * dir, and record a lockfile. Idempotent: an already-satisfied install is a
 * no-op. `checkTool` verifies installed state vs the registry for drift.
 *
 * Supply chain (ADR 0006 D6): a mismatched checksum aborts the install (never
 * runs a binary we didn't pin). Fetch plumbing is `curl` + `tar`/`unzip` — no
 * npm deps. The pod backend (bin-apply.js) uses the same install-dir precedence.
 *
 * Install location (ADR 0006 D3, refined 2026-09-08 to unify dev+pod): prefer
 * ~/.local/bin when it's on PATH; else an existing writable HOME dir already on
 * PATH; else ~/.local/bin with a warning. Lockfiles live under the managed state
 * dir and record each tool's actual `target`, so GC/drift work wherever it landed.
 */
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const { parseBinRegistry, frontmatterBlock, parseRequires } = require('./parse');

const stateRoot = (HOME) => path.join(HOME, '.agents-shared-capabilities', 'state');
const lockDir = (HOME) => path.join(stateRoot(HOME), 'bin-lock');
const lockPath = (HOME, name) => path.join(lockDir(HOME), name + '.json');

// Registry platform keys are `<os>-<arch>`: node's darwin→macos, x64→amd64;
// linux and arm64 pass through. Anything else stays verbatim (→ 'unsupported').
function platformKey() {
  const plat = process.platform === 'darwin' ? 'macos' : process.platform;
  const arch = process.arch === 'x64' ? 'amd64' : process.arch;
  return `${plat}-${arch}`;
}

function sha256File(f) {
  return crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
}

// The dir binaries land in — shared precedence with the pod's bin-apply.js so dev
// and pod agree. Side effect: may create ~/.local/bin and warn if it's off PATH.
function installDir(HOME) {
  if (process.env.BIN_INSTALL_DIR) return process.env.BIN_INSTALL_DIR;
  const parts = (process.env.PATH || '').split(path.delimiter);
  const local = path.join(HOME, '.local', 'bin');
  const canWrite = (d) => { try { fs.accessSync(d, fs.constants.W_OK); return true; } catch (_) { return false; } };
  if (parts.includes(local)) { fs.mkdirSync(local, { recursive: true }); if (canWrite(local)) return local; }
  for (const d of parts) { if (d && d.startsWith(HOME) && fs.existsSync(d) && canWrite(d)) return d; }
  fs.mkdirSync(local, { recursive: true });
  if (!parts.includes(local)) console.log(`bin: WARN ${local} not on PATH — add it so required CLIs resolve`);
  return local;
}

// catalog + personal bin registries → name→tool map (personal overrides catalog).
function loadBinTools(REPO, base) {
  const load = (f) => (fs.existsSync(f) ? parseBinRegistry(fs.readFileSync(f, 'utf8')) : []);
  const map = new Map();
  for (const t of load(path.join(REPO, 'bin', 'registry.yaml'))) map.set(t.name, t);
  if (base) for (const t of load(path.join(base, 'bin', 'registry.yaml'))) map.set(t.name, t);
  return map;
}

// enabled skills → union of their SKILL.md `requires` tool names (the closure).
function requiredToolNames(enable, REPO, base) {
  const names = new Set();
  for (const s of enable.skills || []) {
    const dir = s.source === 'personal' && base
      ? path.join(base, 'skills', s.name)
      : path.join(REPO, 'skills', s.name);
    const sk = path.join(dir, 'SKILL.md');
    if (!fs.existsSync(sk)) continue;
    for (const r of parseRequires(frontmatterBlock(fs.readFileSync(sk, 'utf8')))) names.add(r.name);
  }
  return [...names];
}

function readLock(HOME, name) {
  try { return JSON.parse(fs.readFileSync(lockPath(HOME, name), 'utf8')); } catch (_) { return null; }
}

function urlFor(tool, spec) {
  return String(tool.url || '')
    .replace(/\$\{version\}/g, tool['pinned-version'])
    .replace(/\$\{asset\}/g, spec.asset);
}

// download → verify sha256 (abort on mismatch) → temp path.
function fetchVerified(url, sha256, dest) {
  execFileSync('curl', ['-sSfL', '--max-time', '180', '-o', dest, url], { stdio: ['ignore', 'ignore', 'pipe'] });
  const got = sha256File(dest);
  if (got.toLowerCase() !== String(sha256).toLowerCase()) {
    fs.rmSync(dest, { force: true });
    throw new Error(`checksum mismatch: expected ${sha256}, got ${got}`);
  }
  return dest;
}

function findFile(root, name) {
  for (const e of fs.readdirSync(root, { withFileTypes: true })) {
    const p = path.join(root, e.name);
    if (e.isDirectory()) { const f = findFile(p, name); if (f) return f; }
    else if (e.name === name) return p;
  }
  return null;
}

// extract `binName` out of `arc` (tar.gz|zip|raw) into `destDir` → its path.
function extractBinary(arc, archive, binName, destDir) {
  fs.mkdirSync(destDir, { recursive: true });
  if (archive === 'raw') { const dst = path.join(destDir, binName); fs.copyFileSync(arc, dst); return dst; }
  if (archive === 'tar.gz') execFileSync('tar', ['-xzf', arc, '-C', destDir]);
  else if (archive === 'zip') execFileSync('unzip', ['-oq', arc, '-d', destDir]);
  else throw new Error(`unknown archive type: ${archive}`);
  const found = findFile(destDir, binName);
  if (!found) throw new Error(`binary '${binName}' not found inside ${path.basename(arc)}`);
  return found;
}

// Install one tool for the current platform. Idempotent (uses the recorded target).
// → {name, status: up-to-date|installed|updated|would-install|unsupported, detail}
function installTool(tool, HOME, { dry = false, destDir = null } = {}) {
  const pk = platformKey();
  const spec = (tool.platforms || {})[pk];
  const binName = tool.bin || tool.name;
  if (!spec) return { name: tool.name, status: 'unsupported', detail: `no asset for platform ${pk}` };
  const lock = readLock(HOME, tool.name);
  const satisfied = lock && lock['pinned-version'] === tool['pinned-version'] &&
    lock['asset-sha256'] === spec.sha256 && lock.target &&
    fs.existsSync(lock.target) && sha256File(lock.target) === lock['bin-sha256'];
  if (satisfied) return { name: tool.name, status: 'up-to-date', detail: `${tool['pinned-version']} @ ${lock.target}` };
  if (dry) return { name: tool.name, status: 'would-install', detail: `${tool['pinned-version']} (${pk})` };

  const dir = destDir || installDir(HOME);
  const target = path.join(dir, binName);
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'binfetch-'));
  try {
    const arc = fetchVerified(urlFor(tool, spec), spec.sha256, path.join(tmpRoot, spec.asset));
    const extracted = extractBinary(arc, tool.archive || 'tar.gz', binName, path.join(tmpRoot, 'x'));
    fs.mkdirSync(dir, { recursive: true });
    fs.copyFileSync(extracted, target);
    fs.chmodSync(target, 0o755);
    fs.mkdirSync(lockDir(HOME), { recursive: true });
    fs.writeFileSync(lockPath(HOME, tool.name), JSON.stringify({
      name: tool.name, 'pinned-version': tool['pinned-version'], platform: pk,
      asset: spec.asset, 'asset-sha256': spec.sha256, 'bin-sha256': sha256File(target), target, bin: binName,
    }, null, 2) + '\n');
    return { name: tool.name, status: lock ? 'updated' : 'installed', detail: `${tool['pinned-version']} (${pk}) → ${target}` };
  } finally {
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }
}

// Verify installed state vs the registry (no network). Uses the recorded target.
// → {name, status: ok|missing|stale|tampered|unsupported, detail}
function checkTool(tool, HOME) {
  const pk = platformKey();
  const spec = (tool.platforms || {})[pk];
  const lock = readLock(HOME, tool.name);
  if (!spec) return { name: tool.name, status: 'unsupported', detail: `no asset for platform ${pk}` };
  if (!lock || !lock.target) return { name: tool.name, status: 'missing', detail: 'not installed (no lockfile)' };
  if (!fs.existsSync(lock.target)) return { name: tool.name, status: 'missing', detail: `binary gone: ${lock.target}` };
  if (lock['pinned-version'] !== tool['pinned-version'] || lock['asset-sha256'] !== spec.sha256)
    return { name: tool.name, status: 'stale', detail: `installed ${lock['pinned-version']} but registry pins ${tool['pinned-version']}` };
  if (sha256File(lock.target) !== lock['bin-sha256'])
    return { name: tool.name, status: 'tampered', detail: `on-disk sha256 != lock (${lock.target})` };
  return { name: tool.name, status: 'ok', detail: `${tool['pinned-version']} @ ${lock.target}` };
}

// GC: remove any managed binary/lock whose tool is no longer in `keepNames`.
// → [{name, removed:[paths]}]
function gcTools(keepNames, HOME, { dry = false } = {}) {
  const keep = new Set(keepNames);
  const removed = [];
  const ld = lockDir(HOME);
  if (!fs.existsSync(ld)) return removed;
  for (const f of fs.readdirSync(ld)) {
    if (!f.endsWith('.json')) continue;
    const name = f.replace(/\.json$/, '');
    if (keep.has(name)) continue;
    const lock = readLock(HOME, name);
    const paths = [lockPath(HOME, name)];
    if (lock && lock.target) paths.push(lock.target);
    if (!dry) for (const p of paths) fs.rmSync(p, { force: true });
    removed.push({ name, removed: paths });
  }
  return removed;
}

module.exports = {
  platformKey, installDir, lockDir, loadBinTools, requiredToolNames,
  installTool, checkTool, gcTools,
};
