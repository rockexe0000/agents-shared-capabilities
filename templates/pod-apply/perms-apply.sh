#!/bin/sh
# Pod execution-authorization projector (閘 2), ADR 0007 — NO NODE assumed. MERGES
# the rendered authz fragment (authz-<runtime>.json: {"allow":[...],"deny":[...]},
# produced off-pod by `sync.js --render`) into the runtime's native permission
# config, preserving every other setting (agy rejects unparseable settings, and
# clobbering model/trustedWorkspaces would break the agent — see agy CHANGELOG).
#
# MERGE semantics: permissions.allow is REPLACED by the managed set (the projection
# owns the allowlist — GC-correct: a revoked command disappears); permissions.deny
# is the UNION of existing ∪ managed (never auto-drop a deny — deny > allow). Every
# other key (model, trustedWorkspaces, defaultMode, other permissions.* subkeys) is
# untouched.
#
# JSON is edited with a real parser (python3 → jq → node, first available). If none
# is present the merge is SKIPPED unless the target is absent, in which case a fresh
# file with just the managed permissions is written. Non-fatal overall.
# Params (env): AUTHZ_FILE (default $SRC/authz-claude-code.json), SETTINGS_FILE
# (default ~/.claude/settings.json; antigravity sets the agy settings path).
# Canonical source: agents-shared-capabilities templates/pod-apply/. ADR 0002/0007.
set -u
SRC="${MCP_ARTIFACT_DIR:-/etc/openab/mcp}"
AUTHZ="${AUTHZ_FILE:-$SRC/authz-claude-code.json}"
SETTINGS="${SETTINGS_FILE:-$HOME/.claude/settings.json}"
[ -f "$AUTHZ" ] || { echo "perms-apply: no $AUTHZ — nothing to do"; exit 0; }
mkdir -p "$(dirname "$SETTINGS")"

merge_python() {
  python3 - "$AUTHZ" "$SETTINGS" <<'PY'
import json, sys, os
authz_path, settings_path = sys.argv[1], sys.argv[2]
with open(authz_path) as f:
    managed = json.load(f)
allow = managed.get("allow", []) or []
deny  = managed.get("deny", []) or []
cfg = {}
if os.path.exists(settings_path):
    with open(settings_path) as f:
        txt = f.read().strip()
    cfg = json.loads(txt) if txt else {}
    try:
        open(settings_path + ".bak", "w").write(txt)
    except Exception:
        pass
perms = cfg.get("permissions")
if not isinstance(perms, dict):
    perms = {}
perms["allow"] = allow  # replace: projection owns the allowlist (GC-correct)
existing_deny = perms.get("deny") if isinstance(perms.get("deny"), list) else []
perms["deny"] = sorted(set(existing_deny) | set(deny))  # union: never drop a deny
cfg["permissions"] = perms
with open(settings_path, "w") as f:
    f.write(json.dumps(cfg, indent=2) + "\n")
print("perms-apply: merged via python3 — allow=%s deny=%s -> %s" % (allow, perms["deny"], settings_path))
PY
}

merge_jq() {
  [ -f "$SETTINGS" ] && cp "$SETTINGS" "$SETTINGS.bak"
  base="$SETTINGS"; [ -f "$base" ] || base=/dev/null
  tmp=$(mktemp)
  # --slurpfile so an absent/empty settings yields [] (→ {}) rather than mis-slurping
  # input order. Replace allow with managed; union deny (existing ∪ managed), unique.
  if jq -n --slurpfile cfgArr "$base" --slurpfile mArr "$AUTHZ" '
      ($cfgArr[0] // {}) as $cfg | ($mArr[0] // {}) as $m |
      $cfg + { permissions:
        (($cfg.permissions // {}) as $p |
         $p + { allow: ($m.allow // []),
                deny: (((($p.deny // []) + ($m.deny // [])) | unique)) }) }
    ' > "$tmp" 2>/dev/null && [ -s "$tmp" ]; then
    mv "$tmp" "$SETTINGS"
    echo "perms-apply: merged via jq -> $SETTINGS"
  else
    rm -f "$tmp"; return 1
  fi
}

merge_node() {
  node -e '
    const fs = require("fs");
    const [authzP, settingsP] = [process.argv[1], process.argv[2]];
    const m = JSON.parse(fs.readFileSync(authzP, "utf8"));
    let cfg = {};
    if (fs.existsSync(settingsP)) {
      const txt = fs.readFileSync(settingsP, "utf8");
      cfg = txt.trim() ? JSON.parse(txt) : {};
      try { fs.writeFileSync(settingsP + ".bak", txt); } catch (_) {}
    }
    const p = (cfg.permissions && typeof cfg.permissions === "object") ? cfg.permissions : {};
    p.allow = m.allow || [];
    const ed = Array.isArray(p.deny) ? p.deny : [];
    p.deny = [...new Set([...ed, ...(m.deny || [])])].sort();
    cfg.permissions = p;
    fs.writeFileSync(settingsP, JSON.stringify(cfg, null, 2) + "\n");
    console.log(`perms-apply: merged via node -> ${settingsP}`);
  ' "$AUTHZ" "$SETTINGS"
}

if command -v python3 >/dev/null 2>&1; then
  merge_python && exit 0
fi
if command -v jq >/dev/null 2>&1; then
  merge_jq && exit 0
fi
if command -v node >/dev/null 2>&1; then
  merge_node && exit 0
fi

# No JSON tool present on the image. Parsing/merging JSON safely in pure sh is
# fragile, and a broken settings.json takes agy down — so do NOT attempt it. Skip
# (non-fatal); an un-projected allowlist just means the runtime keeps asking. This
# should not happen on the real runtimes (claude image has node; the debian agy
# image has python3/jq) — if it does, first-verify (ADR 0007) surfaces it.
echo "perms-apply: WARN no python3/jq/node on this image — SKIPPED (authz not projected; runtime will keep prompting)"
exit 0
