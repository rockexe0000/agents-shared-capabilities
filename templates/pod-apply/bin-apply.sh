#!/bin/sh
# Pod bin installer — NO NODE (POSIX sh + curl/tar/unzip/sha256sum), so it runs on
# non-node images too (e.g. the debian antigravity image). Reads bin-install.tsv
# (rendered off-pod by `sync.js --render`), picks this platform's row, fetches the
# pinned asset, verifies sha256 (mismatch aborts that tool — never runs an unpinned
# binary), extracts, installs into $BIN_INSTALL_DIR. Idempotent via a per-tool
# lockfile. Non-fatal overall. Canonical source: agents-shared-capabilities
# templates/pod-apply/ — copied into each overlay (like mcp-apply). ADR 0002/0006.
set -u
SRC="${MCP_ARTIFACT_DIR:-/etc/openab/mcp}"
TSV="$SRC/bin-install.tsv"
DEST="${BIN_INSTALL_DIR:-$HOME/.local/bin}"
LOCKDIR="$HOME/.agents-shared-capabilities/state/bin-lock"
TAB=$(printf '\t')
[ -f "$TSV" ] || { echo "bin-apply: no $TSV — nothing to do"; exit 0; }

os=$(uname -s); arch=$(uname -m)
case "$os" in Linux) os=linux ;; Darwin) os=macos ;; esac
case "$arch" in x86_64|amd64) arch=amd64 ;; aarch64|arm64) arch=arm64 ;; esac
PLAT="$os-$arch"
mkdir -p "$DEST" "$LOCKDIR"
echo "bin-apply: platform $PLAT, dest $DEST"

sha_of() { sha256sum "$1" 2>/dev/null | cut -d' ' -f1; }

while IFS="$TAB" read -r name plat url sha archive bin; do
  { [ -n "${name:-}" ] && [ "$plat" = "$PLAT" ]; } || continue
  target="$DEST/$bin"
  lock="$LOCKDIR/$name"           # contents: "<asset-sha> <bin-sha> <target>"
  if [ -f "$lock" ] && [ -f "$target" ]; then
    # shellcheck disable=SC2046
    set -- $(cat "$lock")
    if [ "${1:-}" = "$sha" ] && [ "${3:-}" = "$target" ] && [ "$(sha_of "$target")" = "${2:-}" ]; then
      echo "bin-apply: $name up-to-date → $target"; continue
    fi
  fi
  tmp=$(mktemp -d); asset="$tmp/dl"
  if ! curl -sSfL --max-time 180 -o "$asset" "$url"; then
    echo "bin-apply: $name download failed — skipped"; rm -rf "$tmp"; continue
  fi
  got=$(sha_of "$asset")
  if [ "$got" != "$sha" ]; then
    echo "bin-apply: $name CHECKSUM MISMATCH (want $sha got $got) — skipped"; rm -rf "$tmp"; continue
  fi
  ex="$tmp/x"; mkdir -p "$ex"
  case "$archive" in
    tar.gz) tar -xzf "$asset" -C "$ex" ;;
    zip)    unzip -oq "$asset" -d "$ex" ;;
    raw)    cp "$asset" "$ex/$bin" ;;
    *) echo "bin-apply: $name unknown archive '$archive' — skipped"; rm -rf "$tmp"; continue ;;
  esac
  found=$(find "$ex" -type f -name "$bin" 2>/dev/null | head -n1)
  if [ -z "$found" ]; then
    echo "bin-apply: $name binary '$bin' not in archive — skipped"; rm -rf "$tmp"; continue
  fi
  cp "$found" "$target"; chmod 755 "$target"
  printf '%s %s %s\n' "$sha" "$(sha_of "$target")" "$target" > "$lock"
  echo "bin-apply: $name installed → $target"
  rm -rf "$tmp"
done < "$TSV"

# GC: drop managed tools no longer in the manifest.
required=$(cut -f1 "$TSV" | sort -u)
for lf in "$LOCKDIR"/*; do
  [ -f "$lf" ] || continue
  nm=$(basename "$lf")
  printf '%s\n' "$required" | grep -qx "$nm" && continue
  # shellcheck disable=SC2046
  set -- $(cat "$lf"); rm -f "${3:-}" "$lf"
  echo "bin-apply: $nm removed (no longer required)"
done

case ":$PATH:" in
  *":$DEST:"*) : ;;
  *) echo "bin-apply: WARN $DEST not on PATH — add it so required CLIs resolve" ;;
esac
