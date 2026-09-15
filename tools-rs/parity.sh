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

if [ "$MODE" = "update-golden" ]; then echo "golden regenerated."; exit 0; fi
if [ "$fail" -ne 0 ]; then echo "PARITY FAILED"; exit 1; fi
echo "PARITY OK ($MODE)"
