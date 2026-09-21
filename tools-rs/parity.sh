#!/usr/bin/env bash
# parity.sh — ADR 0008 regression gate: prove `capsync` renders/checks/syncs/applies the
# catalog exactly as the committed reference expects.
#
# History: during the migration this diffed rust vs the node `tools/sync.js` reference vs the
# committed golden. Once per-axis byte parity held, the node `tools/` backend was retired
# (ADR 0008 Phase 4 tail), so the gate is now **rust vs committed golden** — a pure regression
# check on the single Rust implementation.
#
# For every fixture under tools-rs/tests/fixtures/<case>/agent/{capabilities,permissions}.md it
# renders the artifacts with `capsync --render` and diffs them against tests/golden/<case>/*.
# It then exercises the stateful paths (--check, live `sync`, --check-tools, `sync --with-tools`,
# `apply`) in isolated $HOMEs and asserts capsync's behaviour directly.
#
# Compared files: authz-*.json + authz-suggest.txt + openab-agent-mcp.json / runtime-mcp.json
# + bin-install.tsv + skills.tar.b64 / skills.list.
#
# Modes:
#   (default)        build + run capsync; diff render vs golden + run the stateful assertions.
#   --update-golden  re-render the committed golden from capsync (maintenance).
#
# Needs a C linker for `cargo build` (CI's ubuntu-latest ships one). A linker-less dev box can
# only run `cargo check`/`fmt`/`clippy`, not this gate.
#
# Exit 0 = gate holds; non-zero = a diff/assertion failed (prints it).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
FIXTURES="$HERE/tests/fixtures"
GOLDEN="$HERE/tests/golden"
FILES=(authz-antigravity.json authz-claude-code.json authz-suggest.txt bin-install.tsv openab-agent-mcp.json runtime-mcp.json skills.list skills.tar.b64)

MODE="full"
case "${1:-}" in
  --update-golden) MODE="update-golden" ;;
  "") : ;;
  *) echo "usage: parity.sh [--update-golden]" >&2; exit 2 ;;
esac

echo "== building rust capsync (release) =="
( cd "$HERE" && cargo build --release --quiet )
RUST_BIN="$HERE/target/release/capsync"

fail=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for capdir in "$FIXTURES"/*/; do
  case="$(basename "$capdir")"
  cap="$capdir/agent/capabilities.md"
  [ -f "$cap" ] || { echo "SKIP $case (no capabilities.md)"; continue; }

  rustout="$tmp/rust/$case"; mkdir -p "$rustout"
  # capsync takes the catalog explicitly (a standalone binary can't infer it from __dirname).
  "$RUST_BIN" --render "$rustout" --capabilities "$cap" --catalog "$REPO" >/dev/null

  if [ "$MODE" = "update-golden" ]; then
    mkdir -p "$GOLDEN/$case"
    for f in "${FILES[@]}"; do cp "$rustout/$f" "$GOLDEN/$case/$f"; done
    echo "updated golden: $case"
    continue
  fi

  for f in "${FILES[@]}"; do
    g="$GOLDEN/$case/$f"
    if [ ! -f "$g" ]; then echo "MISSING golden: $case/$f"; fail=1; continue; fi
    if ! diff -u "$g" "$rustout/$f"; then echo "DRIFT rust vs golden: $case/$f"; fail=1; fi
  done

  # --check: re-render + diff the committed golden → must report IN SYNC (exit 0).
  if ! "$RUST_BIN" --check "$GOLDEN/$case" --capabilities "$cap" --catalog "$REPO" >/dev/null 2>&1; then
    echo "CHECK: false drift on in-sync golden: $case"; fail=1
  fi
  echo "ok: $case"
done

if [ "$MODE" = "update-golden" ]; then echo "golden regenerated."; exit 0; fi

# --check negative: an empty dir has none of the artifacts → must report drift (exit != 0).
emptydir="$tmp/emptycheck"; mkdir -p "$emptydir"
negcap="$FIXTURES/empty/agent/capabilities.md"
if [ -f "$negcap" ]; then
  if "$RUST_BIN" --check "$emptydir" --capabilities "$negcap" --catalog "$REPO" >/dev/null 2>&1; then
    echo "CHECK: expected drift on empty dir, got exit 0"; fail=1
  fi
  echo "ok: --check drift (negative)"
fi

# ---- live sync: SKILLS symlink axis (Phase 1e-1) ----
# Isolated $HOME enabling a catalog skill; `capsync sync` must symlink it into each runtime's
# skills dir.
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
hr="$tmp/synchome_r"; make_synchome "$hr"
HOME="$hr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
lr_out="$(collect_links "$hr")"
if ! printf '%s\n' "$lr_out" | grep -q "\.claude/skills/cfdrop -> $REPO/skills/cfdrop"; then
  echo "SYNC: cfdrop not linked as expected:"; printf '%s\n' "$lr_out"; fail=1
fi
echo "ok: sync (skills symlink)"

# ---- sync MCP facade axis (Phase 1e-2a) ----
# Enable a catalog facade server (octobroker) with a PRE-EXISTING ~/.openab/agent/mcp.json
# (extra top-level key + a preexisting server) → exercises the read-modify-write merge.
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
fhr="$tmp/facadehome_r"; make_facadehome "$fhr"
HOME="$fhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
if ! grep -q '"octobroker"' "$fhr/.openab/agent/mcp.json" || ! grep -q '"keep-me"' "$fhr/.openab/agent/mcp.json"; then
  echo "SYNC: facade mcp.json missing octobroker or dropped existing key"; cat "$fhr/.openab/agent/mcp.json"; fail=1
fi
echo "ok: sync (mcp facade)"

# ---- sync MCP direct axis (Phase 1e-2b/2c) ----
# Enable two catalog direct servers (example-fs stdio + oab-facade http), no secrets set →
# env:/${} resolve to "". Must land in claude/antigravity/opencode configs + codex config.toml.
make_directhome() {
  local h="$1"
  mkdir -p "$h/personal" "$h/.claude" "$h/.codex" "$h/.gemini/config" "$h/.config/opencode"
  printf 'skills:\nmcp:\n  servers:\n    - name: example-fs\n    - name: oab-facade\nhooks:\n' \
    > "$h/personal/capabilities.md"
}
dhr="$tmp/directhome_r"; make_directhome "$dhr"
HOME="$dhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
DIRECT_FILES=(".claude.json" ".gemini/config/mcp_config.json" ".config/opencode/opencode.json" ".codex/config.toml")
for f in "${DIRECT_FILES[@]}"; do
  # example-fs appears quoted in JSON, as [mcp_servers.example-fs] in codex TOML.
  if ! grep -q 'example-fs' "$dhr/$f" 2>/dev/null; then
    echo "SYNC: direct $f missing example-fs"; fail=1
  fi
done
echo "ok: sync (mcp direct)"

# ---- sync hooks axis (Phase 1e-3) ----
# Enable a catalog hook (post-edit-noop). claude settings.json is PRE-SEEDED with a user-authored
# hook + a stale managed entry → exercises strip-managed + keep-user + re-add.
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
hhr="$tmp/hookhome_r"; make_hookhome "$hhr"
HOME="$hhr" "$RUST_BIN" sync --catalog "$REPO" >/dev/null 2>&1
if ! grep -q 'post-edit-noop' "$hhr/.claude/settings.json" || ! grep -q 'user-hook' "$hhr/.claude/settings.json" \
   || grep -q '"command": "stale"' "$hhr/.claude/settings.json"; then
  echo "SYNC: claude hooks strip/keep/add wrong"; cat "$hhr/.claude/settings.json"; fail=1
fi
echo "ok: sync (hooks)"

# ---- --check-tools (Phase 1f-1) ----
# Hermetic bin drift check: a personal skill requires a personal bin tool; a hand-crafted
# lockfile + fake installed binary. No network. In-sync → exit 0; after tampering → drift (1).
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
thr="$tmp/toolhome_r"; make_toolhome "$thr"
# capture exit codes without tripping `set -e` (drift returns non-zero by design).
r_ok=0; HOME="$thr" "$RUST_BIN" --check-tools --capabilities "$thr/personal/capabilities.md" --catalog "$REPO" >/dev/null 2>&1 || r_ok=$?
[ "$r_ok" -eq 0 ] || { echo "CHECK-TOOLS: expected exit 0 (in sync), got $r_ok"; fail=1; }
printf 'TAMPERED\n' > "$thr/.local/bin/demotool"
r_tam=0; HOME="$thr" "$RUST_BIN" --check-tools --capabilities "$thr/personal/capabilities.md" --catalog "$REPO" >/dev/null 2>&1 || r_tam=$?
[ "$r_tam" -ne 0 ] || { echo "CHECK-TOOLS: expected drift after tamper, got 0"; fail=1; }
echo "ok: --check-tools (in-sync + tamper)"

# ---- sync --with-tools (Phase 1f-2) ----
# Hermetic install: a fake tar.gz served over a file:// URL (no real network). capsync must
# install the binary and write the lockfile.
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
ihr="$tmp/installhome_r"; make_installhome "$ihr"
HOME="$ihr" BIN_INSTALL_DIR="$ihr/installed" "$RUST_BIN" sync --with-tools --catalog "$REPO" >/dev/null 2>&1 || true
r_lock="$ihr/.agents-shared-capabilities/state/bin-lock/demotool.json"
if [ ! -x "$ihr/installed/demotool" ] || [ ! -f "$r_lock" ]; then
  echo "WITH-TOOLS: demotool not installed / no lockfile"; fail=1
fi
echo "ok: sync --with-tools (install)"

# ---- lint (Phase 1h) ----
# valid: the real catalog → exit 0. broken: a temp catalog with a duplicate mcp server name →
# exit 1. (capsync takes --catalog explicitly, so no need to copy the tooling in.)
r_lint=0; "$RUST_BIN" lint --catalog "$REPO" >/dev/null 2>&1 || r_lint=$?
[ "$r_lint" -eq 0 ] || { echo "LINT: real catalog should be valid, got $r_lint"; fail=1; }
bc="$tmp/badcatalog"; mkdir -p "$bc/mcp"
printf 'servers:\n  - name: dup\n  - name: dup\n' > "$bc/mcp/registry.yaml"
r_bad=0; "$RUST_BIN" lint --catalog "$bc" >/dev/null 2>&1 || r_bad=$?
[ "$r_bad" -ne 0 ] || { echo "LINT: broken catalog should fail, got 0"; fail=1; }
echo "ok: lint (valid + broken)"

# ---- apply authz (ADR 0008 Phase 4) ----
# From a rendered authz-claude-code.json, merging into a seeded settings.json (unrelated key
# kept + a pre-existing allow REPLACED + a pre-existing deny UNIONed).
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
ahr="$tmp/applyhome_r"; seed_settings "$ahr"
HOME="$ahr" "$RUST_BIN" apply --from "$ardir" >/dev/null 2>&1 || { echo "APPLY authz: rust apply errored"; fail=1; }
if ! grep -q '"keep-me"' "$ahr/.claude/settings.json" || ! grep -q '"Bash(cfdrop:\*)"' "$ahr/.claude/settings.json"; then
  echo "APPLY authz: didn't merge as expected:"; cat "$ahr/.claude/settings.json"; fail=1
fi
echo "ok: apply (authz)"

# ---- apply mcp (ADR 0008 Phase 4) ----
# Rendered facade + direct MCP artifacts merged into pre-existing targets (unrelated key kept +
# a preexisting server) — per-server merge into ~/.openab/agent/mcp.json + ~/.claude.json.
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
mhr="$tmp/mcphome_r"; seed_mcp "$mhr"
HOME="$mhr" "$RUST_BIN" apply --from "$mrdir" >/dev/null 2>&1 || { echo "APPLY mcp: rust apply errored"; fail=1; }
grep -q '"octobroker"' "$mhr/.openab/agent/mcp.json" && grep -q '"keep-me"' "$mhr/.openab/agent/mcp.json" \
  || { echo "APPLY mcp: facade merge wrong"; cat "$mhr/.openab/agent/mcp.json"; fail=1; }
grep -q '"oab-facade"' "$mhr/.claude.json" && grep -q '"keep-me-too"' "$mhr/.claude.json" \
  || { echo "APPLY mcp: .claude.json merge wrong"; cat "$mhr/.claude.json"; fail=1; }
echo "ok: apply (mcp)"

# ---- apply bin (ADR 0008 Phase 4) ----
# Rendered bin-install.tsv pointing at the file:// fake asset (reused from --with-tools):
# `capsync apply` must install the binary into BIN_INSTALL_DIR.
brdir="$tmp/bin_render"; mkdir -p "$brdir"
printf 'demotool\t%s\tfile://%s\t%s\ttar.gz\tdemotool\n' "$pk" "$fake/demotool-$pk.tar.gz" "$asset_sha" > "$brdir/bin-install.tsv"
bhr="$tmp/binhome_r"; mkdir -p "$bhr/installed"
HOME="$bhr" BIN_INSTALL_DIR="$bhr/installed" "$RUST_BIN" apply --from "$brdir" >/dev/null 2>&1 || { echo "APPLY bin: rust apply errored"; fail=1; }
[ -x "$bhr/installed/demotool" ] || { echo "APPLY bin: didn't install demotool"; fail=1; }
echo "ok: apply (bin)"

# ---- apply skills (ADR 0008 Phase 4) ----
# Rendered skills.tar.b64 + skills.list → `capsync apply` extracts the skills tree into SKILLS_DIR.
srdir="$tmp/skills_render"; mkdir -p "$srdir/stage/demo-skill"
printf 'hello\n' > "$srdir/stage/demo-skill/SKILL.md"
( cd "$srdir/stage" && tar -cf - demo-skill | base64 > "$srdir/skills.tar.b64" )
printf 'demo-skill\n' > "$srdir/skills.list"
shr="$tmp/skillshome_r"; mkdir -p "$shr"
HOME="$shr" SKILLS_DIR="$shr/skills" "$RUST_BIN" apply --from "$srdir" >/dev/null 2>&1 || { echo "APPLY skills: rust apply errored"; fail=1; }
[ -f "$shr/skills/demo-skill/SKILL.md" ] || { echo "APPLY skills: didn't extract"; fail=1; }
echo "ok: apply (skills)"

if [ "$fail" -ne 0 ]; then echo "GATE FAILED"; exit 1; fi
echo "GATE OK"
