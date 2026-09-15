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

if [ "$MODE" = "update-golden" ]; then echo "golden regenerated."; exit 0; fi
if [ "$fail" -ne 0 ]; then echo "PARITY FAILED"; exit 1; fi
echo "PARITY OK ($MODE)"
