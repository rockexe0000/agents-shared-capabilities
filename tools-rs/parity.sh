#!/usr/bin/env bash
# parity.sh — ADR 0008 Phase 0 gate: prove the Rust `capsync --render` authz axis is
# byte-for-byte identical to `tools/sync.js --render`.
#
# For every fixture under tools-rs/tests/fixtures/<case>/personal/{capabilities,permissions}.md
# it renders the authz artifacts three ways and diffs them:
#   1. node  sync.js --render      -> the reference implementation
#   2. rust  capsync --render      -> the port under test
#   3. tests/golden/<case>/*.json  -> committed reference (catches drift in EITHER side)
#
# Compared files: authz-*.json + authz-suggest.txt (Phase 0/1b) + openab-agent-mcp.json /
# runtime-mcp.json (Phase 1a) + bin-install.tsv (1b) + skills.tar.b64 / skills.list (1c).
# Also asserts `--check` parity (Phase 1d): node & rust --check on the in-sync golden exit 0,
# and both report drift (exit != 0) against an empty dir.
#
# Modes:
#   (default)      build+run rust and node, diff all three. Needs a C linker for cargo.
#   --node-only    skip the rust side (diff node vs golden only). For environments with
#                  no linker; the rust half then runs in CI where cc is present.
#   --update-golden  re-generate committed golden from node output (maintenance).
#
# Exit 0 = parity holds; non-zero = a diff was found (prints it).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SYNC="$REPO/tools/sync.js"
FIXTURES="$HERE/tests/fixtures"
GOLDEN="$HERE/tests/golden"
FILES=(authz-antigravity.json authz-claude-code.json authz-suggest.txt bin-install.tsv openab-agent-mcp.json runtime-mcp.json skills.list skills.tar.b64)

MODE="full"
case "${1:-}" in
  --node-only) MODE="node-only" ;;
  --update-golden) MODE="update-golden" ;;
  "") : ;;
  *) echo "usage: parity.sh [--node-only|--update-golden]" >&2; exit 2 ;;
esac

RUST_BIN=""
if [ "$MODE" = "full" ]; then
  echo "== building rust capsync (release) =="
  ( cd "$HERE" && cargo build --release --quiet )
  RUST_BIN="$HERE/target/release/capsync"
fi

fail=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for capdir in "$FIXTURES"/*/; do
  case="$(basename "$capdir")"
  cap="$capdir/agent/capabilities.md"
  [ -f "$cap" ] || { echo "SKIP $case (no capabilities.md)"; continue; }

  nodeout="$tmp/node/$case"; mkdir -p "$nodeout"
  node "$SYNC" --render "$nodeout" --capabilities "$cap" >/dev/null 2>&1

  if [ "$MODE" = "update-golden" ]; then
    mkdir -p "$GOLDEN/$case"
    for f in "${FILES[@]}"; do cp "$nodeout/$f" "$GOLDEN/$case/$f"; done
    echo "updated golden: $case"
    continue
  fi

  rustout=""
  if [ "$MODE" = "full" ]; then
    rustout="$tmp/rust/$case"; mkdir -p "$rustout"
    # node infers the catalog (REPO) via __dirname; capsync takes it explicitly (ADR 0008).
    "$RUST_BIN" --render "$rustout" --capabilities "$cap" --catalog "$REPO" >/dev/null
  fi

  for f in "${FILES[@]}"; do
    g="$GOLDEN/$case/$f"
    if [ ! -f "$g" ]; then echo "MISSING golden: $case/$f"; fail=1; continue; fi
    if ! diff -u "$g" "$nodeout/$f"; then echo "DRIFT node vs golden: $case/$f"; fail=1; fi
    if [ "$MODE" = "full" ]; then
      if ! diff -u "$g" "$rustout/$f"; then echo "DRIFT rust vs golden: $case/$f"; fail=1; fi
    fi
  done

  # --check parity: re-render + diff the committed golden → must report IN SYNC (exit 0).
  if ! node "$SYNC" --check "$GOLDEN/$case" --capabilities "$cap" >/dev/null 2>&1; then
    echo "CHECK node: false drift on in-sync golden: $case"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    if ! "$RUST_BIN" --check "$GOLDEN/$case" --capabilities "$cap" --catalog "$REPO" >/dev/null 2>&1; then
      echo "CHECK rust: false drift on in-sync golden: $case"; fail=1
    fi
  fi
  echo "ok: $case"
done

# --check negative: an empty dir has none of the artifacts → must report drift (exit != 0).
emptydir="$tmp/emptycheck"; mkdir -p "$emptydir"
negcap="$FIXTURES/empty/agent/capabilities.md"
if [ -f "$negcap" ]; then
  if node "$SYNC" --check "$emptydir" --capabilities "$negcap" >/dev/null 2>&1; then
    echo "CHECK node: expected drift on empty dir, got exit 0"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    if "$RUST_BIN" --check "$emptydir" --capabilities "$negcap" --catalog "$REPO" >/dev/null 2>&1; then
      echo "CHECK rust: expected drift on empty dir, got exit 0"; fail=1
    fi
  fi
  echo "ok: --check drift (negative)"
fi

# ---- sync (live projection) parity — SKILLS symlink axis (Phase 1e-1) ----
# Isolated $HOME enabling a catalog skill; node vs rust `sync` must produce identical skill
# symlinks across the runtime skills dirs. (MCP/hooks projection compared in later 1e slices.)
if [ "$MODE" != "update-golden" ]; then
  make_synchome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/.claude" "$h/.codex" "$h/.gemini/antigravity-cli" "$h/.config/opencode"
    printf 'skills:\n  - name: cfdrop\n    source: catalog\nmcp:\n  servers:\nhooks:\n' \
      > "$h/personal/capabilities.md"
  }
  collect_links() {
    local h="$1"
    for d in ".claude/skills" ".codex/skills" ".gemini/antigravity-cli/skills" ".config/opencode/skills"; do
      [ -d "$h/$d" ] || continue
      for l in "$h/$d"/*; do [ -L "$l" ] && echo "$d/$(basename "$l") -> $(readlink "$l")"; done
    done | sort
  }
  hn="$tmp/synchome_n"; make_synchome "$hn"
  ( cd "$REPO/tools" && HOME="$hn" node sync.js >/dev/null 2>&1 )
  ln_out="$(collect_links "$hn")"
  if ! printf '%s\n' "$ln_out" | grep -q "\.claude/skills/cfdrop -> $REPO/skills/cfdrop"; then
    echo "SYNC node: cfdrop not linked as expected:"; printf '%s\n' "$ln_out"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    hr="$tmp/synchome_r"; make_synchome "$hr"
    HOME="$hr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
    lr_out="$(collect_links "$hr")"
    if [ "$ln_out" != "$lr_out" ]; then
      echo "SYNC rust vs node symlink drift:"; diff <(printf '%s\n' "$ln_out") <(printf '%s\n' "$lr_out"); fail=1
    fi
  fi
  echo "ok: sync (skills symlink)"

  # ---- sync MCP facade axis (Phase 1e-2a) ----
  # Isolated $HOME enabling a catalog facade server (octobroker), with a PRE-EXISTING
  # ~/.openab/agent/mcp.json (extra top-level key + a preexisting server) so we also exercise
  # the read-modify-write merge. node vs rust must produce a byte-identical mcp.json.
  make_facadehome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/.openab/agent" "$h/.claude" "$h/.codex" "$h/.gemini/antigravity-cli" "$h/.config/opencode"
    printf 'skills:\nmcp:\n  servers:\n    - name: octobroker\nhooks:\n' > "$h/personal/capabilities.md"
    cat > "$h/.openab/agent/mcp.json" <<'JSON'
{
  "someOtherKey": "keep-me",
  "mcpServers": {
    "preexisting": {
      "type": "stdio",
      "command": "foo",
      "args": [],
      "env": {}
    }
  }
}
JSON
  }
  fhn="$tmp/facadehome_n"; make_facadehome "$fhn"
  ( cd "$REPO/tools" && HOME="$fhn" node sync.js >/dev/null 2>&1 )
  if ! grep -q '"octobroker"' "$fhn/.openab/agent/mcp.json" || ! grep -q '"keep-me"' "$fhn/.openab/agent/mcp.json"; then
    echo "SYNC node: facade mcp.json missing octobroker or dropped existing key"; cat "$fhn/.openab/agent/mcp.json"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    fhr="$tmp/facadehome_r"; make_facadehome "$fhr"
    HOME="$fhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
    if ! diff -u "$fhn/.openab/agent/mcp.json" "$fhr/.openab/agent/mcp.json"; then
      echo "SYNC rust vs node facade mcp.json drift"; fail=1
    fi
  fi
  echo "ok: sync (mcp facade)"

  # ---- sync MCP direct axis (Phase 1e-2b/2c) ----
  # Isolated $HOME enabling two catalog direct servers (example-fs stdio + oab-facade http),
  # no secrets set → env:/${} resolve to "". node vs rust must produce byte-identical
  # claude/antigravity/opencode configs + codex config.toml (TOML managed block).
  make_directhome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/.claude" "$h/.codex" "$h/.gemini/config" "$h/.config/opencode"
    printf 'skills:\nmcp:\n  servers:\n    - name: example-fs\n    - name: oab-facade\nhooks:\n' \
      > "$h/personal/capabilities.md"
  }
  dhn="$tmp/directhome_n"; make_directhome "$dhn"
  ( cd "$REPO/tools" && HOME="$dhn" node sync.js >/dev/null 2>&1 )
  DIRECT_FILES=(".claude.json" ".gemini/config/mcp_config.json" ".config/opencode/opencode.json" ".codex/config.toml")
  for f in "${DIRECT_FILES[@]}"; do
    # example-fs appears quoted in JSON, as [mcp_servers.example-fs] in codex TOML.
    if ! grep -q 'example-fs' "$dhn/$f" 2>/dev/null; then
      echo "SYNC node: direct $f missing example-fs"; fail=1
    fi
  done
  if [ "$MODE" = "full" ]; then
    dhr="$tmp/directhome_r"; make_directhome "$dhr"
    HOME="$dhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
    for f in "${DIRECT_FILES[@]}"; do
      if ! diff -u "$dhn/$f" "$dhr/$f"; then echo "SYNC rust vs node direct drift: $f"; fail=1; fi
    done
  fi
  echo "ok: sync (mcp direct)"

  # ---- sync hooks axis (Phase 1e-3) ----
  # Enable a catalog hook (post-edit-noop). claude settings.json is PRE-SEEDED with a
  # user-authored hook + a stale managed entry → exercises strip-managed + keep-user + re-add;
  # antigravity hooks.json is fresh. node vs rust must produce byte-identical files.
  make_hookhome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/.claude" "$h/.gemini/config"
    printf 'skills:\nmcp:\n  servers:\nhooks:\n  - name: post-edit-noop\n    effect: allow\n' \
      > "$h/personal/capabilities.md"
    cat > "$h/.claude/settings.json" <<'JSON'
{
  "model": "sonnet",
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "user-hook" } ] }
    ],
    "PostToolUse": [
      { "matcher": "Edit|Write", "hooks": [ { "type": "command", "command": "stale", "_managedBy": "agents-shared-capabilities", "_hook": "old" } ] }
    ]
  }
}
JSON
  }
  hhn="$tmp/hookhome_n"; make_hookhome "$hhn"
  ( cd "$REPO/tools" && HOME="$hhn" node sync.js >/dev/null 2>&1 )
  HOOK_FILES=(".claude/settings.json" ".gemini/config/hooks.json")
  if ! grep -q 'post-edit-noop' "$hhn/.claude/settings.json" || ! grep -q 'user-hook' "$hhn/.claude/settings.json" \
     || grep -q '"command": "stale"' "$hhn/.claude/settings.json"; then
    echo "SYNC node: claude hooks strip/keep/add wrong"; cat "$hhn/.claude/settings.json"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    hhr="$tmp/hookhome_r"; make_hookhome "$hhr"
    HOME="$hhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
    for f in "${HOOK_FILES[@]}"; do
      if ! diff -u "$hhn/$f" "$hhr/$f"; then echo "SYNC rust vs node hooks drift: $f"; fail=1; fi
    done
  fi
  echo "ok: sync (hooks)"
fi

# ---- --check-tools parity (Phase 1f-1) ----
# Hermetic bin drift check: a personal skill requires a personal bin tool; a hand-crafted
# lockfile + fake installed binary. No network. node vs rust must agree on the exit code:
# OK (matching lock) → 0; after tampering the binary → drift (1). Cross-checks that rust's
# SHA-256 == node crypto == sha256sum.
if [ "$MODE" != "update-golden" ]; then
  sha256of() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
  case "$(uname -s)" in Linux) tos=linux;; Darwin) tos=macos;; *) tos=$(uname -s);; esac
  case "$(uname -m)" in x86_64) tarch=amd64;; aarch64|arm64) tarch=arm64;; *) tarch=$(uname -m);; esac
  pk="$tos-$tarch"
  make_toolhome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/skills/demo-skill" "$h/bin" \
      "$h/.agents-shared-capabilities/state/bin-lock" "$h/.local/bin"
    printf 'skills:\n  - name: demo-skill\n    source: personal\nmcp:\n  servers:\nhooks:\n' \
      > "$h/personal/capabilities.md"
    printf -- '---\nname: demo-skill\ndescription: x\nrequires:\n  - name: demotool\n---\n# demo\n' \
      > "$h/skills/demo-skill/SKILL.md"
    cat > "$h/bin/registry.yaml" <<REG
tools:
  - name: demotool
    pinned-version: v1
    bin: demotool
    archive: raw
    url: "https://example/demotool"
    platforms:
      $pk:
        asset: demotool-$pk
        sha256: assetsha000
REG
    printf 'FAKEBIN\n' > "$h/.local/bin/demotool"
    local binsha; binsha="$(sha256of "$h/.local/bin/demotool")"
    cat > "$h/.agents-shared-capabilities/state/bin-lock/demotool.json" <<LOCK
{
  "name": "demotool",
  "pinned-version": "v1",
  "platform": "$pk",
  "asset": "demotool-$pk",
  "asset-sha256": "assetsha000",
  "bin-sha256": "$binsha",
  "target": "$h/.local/bin/demotool",
  "bin": "demotool"
}
LOCK
  }
  thn="$tmp/toolhome_n"; make_toolhome "$thn"
  # capture exit codes without tripping `set -e` (drift returns non-zero by design).
  n_ok=0; ( cd "$REPO/tools" && HOME="$thn" node sync.js --check-tools --capabilities "$thn/personal/capabilities.md" >/dev/null 2>&1 ) || n_ok=$?
  [ "$n_ok" -eq 0 ] || { echo "CHECK-TOOLS node: expected exit 0 (in sync), got $n_ok"; fail=1; }
  printf 'TAMPERED\n' > "$thn/.local/bin/demotool"
  n_tam=0; ( cd "$REPO/tools" && HOME="$thn" node sync.js --check-tools --capabilities "$thn/personal/capabilities.md" >/dev/null 2>&1 ) || n_tam=$?
  [ "$n_tam" -ne 0 ] || { echo "CHECK-TOOLS node: expected drift after tamper, got 0"; fail=1; }
  if [ "$MODE" = "full" ]; then
    thr="$tmp/toolhome_r"; make_toolhome "$thr"
    r_ok=0; HOME="$thr" "$RUST_BIN" --check-tools --capabilities "$thr/personal/capabilities.md" --catalog "$REPO" >/dev/null 2>&1 || r_ok=$?
    [ "$r_ok" -eq "$n_ok" ] || { echo "CHECK-TOOLS rust in-sync exit $r_ok != node $n_ok"; fail=1; }
    printf 'TAMPERED\n' > "$thr/.local/bin/demotool"
    r_tam=0; HOME="$thr" "$RUST_BIN" --check-tools --capabilities "$thr/personal/capabilities.md" --catalog "$REPO" >/dev/null 2>&1 || r_tam=$?
    [ "$r_tam" -eq "$n_tam" ] || { echo "CHECK-TOOLS rust tamper exit $r_tam != node $n_tam"; fail=1; }
  fi
  echo "ok: --check-tools (in-sync + tamper)"

  # ---- sync --with-tools parity (Phase 1f-2) ----
  # Hermetic install: a fake tar.gz served over a file:// URL (no real network). node vs rust
  # must install the identical binary and write matching lockfiles (modulo the per-home target).
  fake="$tmp/fakeasset"; mkdir -p "$fake/stage"
  printf '#!/bin/sh\necho demotool\n' > "$fake/stage/demotool"
  ( cd "$fake/stage" && tar -czf "$fake/demotool-$pk.tar.gz" demotool )
  asset_sha="$(sha256of "$fake/demotool-$pk.tar.gz")"
  make_installhome() {
    local h="$1"
    mkdir -p "$h/personal" "$h/skills/demo-skill" "$h/bin" "$h/installed"
    printf 'skills:\n  - name: demo-skill\n    source: personal\nmcp:\n  servers:\nhooks:\n' \
      > "$h/personal/capabilities.md"
    printf -- '---\nname: demo-skill\ndescription: x\nrequires:\n  - name: demotool\n---\n# demo\n' \
      > "$h/skills/demo-skill/SKILL.md"
    cat > "$h/bin/registry.yaml" <<REG
tools:
  - name: demotool
    pinned-version: v1
    bin: demotool
    archive: tar.gz
    url: "file://$fake/\${asset}"
    platforms:
      $pk:
        asset: demotool-$pk.tar.gz
        sha256: $asset_sha
REG
  }
  ihn="$tmp/installhome_n"; make_installhome "$ihn"
  # node: live sync is the default action; --with-tools opts into the bin installer.
  ( cd "$REPO/tools" && HOME="$ihn" BIN_INSTALL_DIR="$ihn/installed" node sync.js --with-tools >/dev/null 2>&1 ) || true
  n_lock="$ihn/.agents-shared-capabilities/state/bin-lock/demotool.json"
  if [ ! -x "$ihn/installed/demotool" ] || [ ! -f "$n_lock" ]; then
    echo "WITH-TOOLS node: demotool not installed / no lockfile"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    ihr="$tmp/installhome_r"; make_installhome "$ihr"
    HOME="$ihr" BIN_INSTALL_DIR="$ihr/installed" "$RUST_BIN" sync --with-tools --catalog "$REPO" >/dev/null 2>&1 || true
    r_lock="$ihr/.agents-shared-capabilities/state/bin-lock/demotool.json"
    if ! diff "$ihn/installed/demotool" "$ihr/installed/demotool"; then echo "WITH-TOOLS installed binary drift"; fail=1; fi
    # lockfiles must match except the per-home absolute target path.
    if ! diff <(grep -v '"target"' "$n_lock") <(grep -v '"target"' "$r_lock"); then
      echo "WITH-TOOLS lockfile drift (modulo target)"; fail=1
    fi
  fi
  echo "ok: sync --with-tools (install)"

  # ---- lint parity (Phase 1h) ----
  # valid: the real catalog → both exit 0. broken: a temp catalog copy with a duplicate mcp
  # server name → both exit 1. (node lint.js infers REPO from __dirname, so we copy tools/ into
  # the temp catalog and run it from there.) Compare exit codes.
  n_lint=0; ( cd "$REPO/tools" && node lint.js >/dev/null 2>&1 ) || n_lint=$?
  [ "$n_lint" -eq 0 ] || { echo "LINT node: real catalog should be valid, got $n_lint"; fail=1; }
  bc="$tmp/badcatalog"; mkdir -p "$bc/mcp"; cp -r "$REPO/tools" "$bc/tools"
  printf 'servers:\n  - name: dup\n  - name: dup\n' > "$bc/mcp/registry.yaml"
  n_bad=0; ( cd "$bc/tools" && node lint.js >/dev/null 2>&1 ) || n_bad=$?
  [ "$n_bad" -ne 0 ] || { echo "LINT node: broken catalog should fail, got 0"; fail=1; }
  if [ "$MODE" = "full" ]; then
    r_lint=0; "$RUST_BIN" lint --catalog "$REPO" >/dev/null 2>&1 || r_lint=$?
    [ "$r_lint" -eq "$n_lint" ] || { echo "LINT rust valid exit $r_lint != node $n_lint"; fail=1; }
    r_bad=0; "$RUST_BIN" lint --catalog "$bc" >/dev/null 2>&1 || r_bad=$?
    [ "$r_bad" -eq "$n_bad" ] || { echo "LINT rust broken exit $r_bad != node $n_bad"; fail=1; }
  fi
  echo "ok: lint (valid + broken)"

  # ---- apply authz parity — capsync apply vs perms-apply.sh (ADR 0008 Phase 4) ----
  # From the same rendered authz-claude-code.json, merging into a seeded settings.json
  # (unrelated key kept + a pre-existing allow REPLACED + a pre-existing deny UNIONed),
  # perms-apply.sh and `capsync apply` must produce a byte-identical ~/.claude/settings.json.
  mk_authz_render() {
    mkdir -p "$1"
    printf '{\n  "allow": [\n    "Bash(cfdrop:*)"\n  ],\n  "deny": [\n    "Bash(rm:*)"\n  ]\n}\n' \
      > "$1/authz-claude-code.json"
  }
  seed_settings() {
    mkdir -p "$1/.claude"
    cat > "$1/.claude/settings.json" <<'JSON'
{
  "model": "keep-me",
  "permissions": {
    "allow": [
      "Bash(old:*)"
    ],
    "deny": [
      "Bash(danger:*)"
    ]
  }
}
JSON
  }
  ardir="$tmp/apply_render"; mk_authz_render "$ardir"
  ahn="$tmp/applyhome_n"; seed_settings "$ahn"
  AUTHZ_FILE="$ardir/authz-claude-code.json" SETTINGS_FILE="$ahn/.claude/settings.json" \
    HOME="$ahn" sh "$REPO/templates/pod-apply/perms-apply.sh" >/dev/null 2>&1
  if ! grep -q '"keep-me"' "$ahn/.claude/settings.json" || ! grep -q '"Bash(cfdrop:\*)"' "$ahn/.claude/settings.json"; then
    echo "APPLY node: perms-apply didn't merge as expected:"; cat "$ahn/.claude/settings.json"; fail=1
  fi
  if [ "$MODE" = "full" ]; then
    ahr="$tmp/applyhome_r"; seed_settings "$ahr"
    HOME="$ahr" "$RUST_BIN" apply --from "$ardir" >/dev/null 2>&1
    if ! diff -u "$ahn/.claude/settings.json" "$ahr/.claude/settings.json"; then
      echo "APPLY rust vs node settings.json drift"; fail=1
    fi
  fi
  echo "ok: apply (authz)"

  # ---- apply mcp parity — capsync apply vs mcp-apply.js (ADR 0008 Phase 4) ----
  # Rendered facade + direct MCP artifacts merged into pre-existing targets (unrelated key
  # kept + a preexisting server) — mcp-apply.js's Object.assign merge vs `capsync apply`
  # must produce byte-identical ~/.openab/agent/mcp.json + ~/.claude.json.
  mk_mcp_render() {
    mkdir -p "$1"
    printf '{\n  "mcpServers": {\n    "octobroker": {\n      "type": "http",\n      "url": "http://127.0.0.1:8079/mcp"\n    }\n  }\n}\n' > "$1/openab-agent-mcp.json"
    printf '{\n  "mcpServers": {\n    "oab-facade": {\n      "type": "http",\n      "url": "http://127.0.0.1:8848/mcp"\n    }\n  }\n}\n' > "$1/runtime-mcp.json"
  }
  seed_mcp() {
    mkdir -p "$1/.openab/agent"
    printf '{\n  "someOtherKey": "keep-me",\n  "mcpServers": {\n    "preexisting": { "type": "stdio", "command": "foo" }\n  }\n}\n' > "$1/.openab/agent/mcp.json"
    printf '{\n  "authState": "keep-me-too",\n  "mcpServers": {}\n}\n' > "$1/.claude.json"
  }
  mrdir="$tmp/mcp_render"; mk_mcp_render "$mrdir"
  mhn="$tmp/mcphome_n"; seed_mcp "$mhn"
  HOME="$mhn" SRC="$mrdir" node -e '
    const fs=require("fs"),path=require("path");
    const HOME=process.env.HOME, SRC=process.env.SRC;
    function merge(t,s){ if(!fs.existsSync(s))return; let cur={}; try{cur=JSON.parse(fs.readFileSync(t,"utf8"))}catch(_){}
      const add=JSON.parse(fs.readFileSync(s,"utf8")); cur.mcpServers=Object.assign(cur.mcpServers||{},add.mcpServers||{});
      fs.mkdirSync(path.dirname(t),{recursive:true}); fs.writeFileSync(t,JSON.stringify(cur,null,2)+"\n"); }
    merge(path.join(HOME,".openab","agent","mcp.json"), path.join(SRC,"openab-agent-mcp.json"));
    merge(path.join(HOME,".claude.json"), path.join(SRC,"runtime-mcp.json"));
  ' >/dev/null 2>&1
  grep -q '"octobroker"' "$mhn/.openab/agent/mcp.json" && grep -q '"keep-me"' "$mhn/.openab/agent/mcp.json" || { echo "APPLY node: mcp merge wrong"; fail=1; }
  if [ "$MODE" = "full" ]; then
    mhr="$tmp/mcphome_r"; seed_mcp "$mhr"
    HOME="$mhr" "$RUST_BIN" apply --from "$mrdir" >/dev/null 2>&1
    for tf in ".openab/agent/mcp.json" ".claude.json"; do
      diff -u "$mhn/$tf" "$mhr/$tf" || { echo "APPLY mcp drift ($tf)"; fail=1; }
    done
  fi
  echo "ok: apply (mcp)"

  # ---- apply bin parity — capsync apply vs bin-apply.sh (ADR 0008 Phase 4) ----
  # Rendered bin-install.tsv pointing at the file:// fake asset (reused from --with-tools):
  # bin-apply.sh vs `capsync apply` must install the identical binary into BIN_INSTALL_DIR.
  brdir="$tmp/bin_render"; mkdir -p "$brdir"
  printf 'demotool\t%s\tfile://%s\t%s\ttar.gz\tdemotool\n' "$pk" "$fake/demotool-$pk.tar.gz" "$asset_sha" > "$brdir/bin-install.tsv"
  bhn="$tmp/binhome_n"; mkdir -p "$bhn/installed"
  HOME="$bhn" MCP_ARTIFACT_DIR="$brdir" BIN_INSTALL_DIR="$bhn/installed" sh "$REPO/templates/pod-apply/bin-apply.sh" >/dev/null 2>&1
  [ -x "$bhn/installed/demotool" ] || { echo "APPLY node: bin-apply didn't install demotool"; fail=1; }
  if [ "$MODE" = "full" ]; then
    bhr="$tmp/binhome_r"; mkdir -p "$bhr/installed"
    HOME="$bhr" BIN_INSTALL_DIR="$bhr/installed" "$RUST_BIN" apply --from "$brdir" >/dev/null 2>&1
    diff "$bhn/installed/demotool" "$bhr/installed/demotool" || { echo "APPLY bin installed-binary drift"; fail=1; }
  fi
  echo "ok: apply (bin)"

  # ---- apply skills parity — capsync apply vs skills-apply.sh (ADR 0008 Phase 4) ----
  # Rendered skills.tar.b64 + skills.list → both extract the identical skills tree (incl. the
  # .catalog-managed marker) into SKILLS_DIR.
  srdir="$tmp/skills_render"; mkdir -p "$srdir/stage/demo-skill"
  printf 'hello\n' > "$srdir/stage/demo-skill/SKILL.md"
  ( cd "$srdir/stage" && tar -cf - demo-skill | base64 > "$srdir/skills.tar.b64" )
  printf 'demo-skill\n' > "$srdir/skills.list"
  shn="$tmp/skillshome_n"; mkdir -p "$shn"
  HOME="$shn" MCP_ARTIFACT_DIR="$srdir" SKILLS_DIR="$shn/skills" sh "$REPO/templates/pod-apply/skills-apply.sh" >/dev/null 2>&1
  [ -f "$shn/skills/demo-skill/SKILL.md" ] || { echo "APPLY node: skills-apply didn't extract"; fail=1; }
  if [ "$MODE" = "full" ]; then
    shr="$tmp/skillshome_r"; mkdir -p "$shr"
    HOME="$shr" SKILLS_DIR="$shr/skills" "$RUST_BIN" apply --from "$srdir" >/dev/null 2>&1
    diff -r "$shn/skills" "$shr/skills" || { echo "APPLY skills tree drift"; fail=1; }
  fi
  echo "ok: apply (skills)"
fi

if [ "$MODE" = "update-golden" ]; then echo "golden regenerated."; exit 0; fi
if [ "$fail" -ne 0 ]; then echo "PARITY FAILED"; exit 1; fi
echo "PARITY OK ($MODE)"
