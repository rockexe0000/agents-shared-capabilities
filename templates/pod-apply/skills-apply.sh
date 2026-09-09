#!/bin/sh
# Pod skill projector — NO NODE (POSIX sh + base64/tar). Extracts the enabled skills
# from skills.tar.b64 (rendered off-pod by `sync.js --render`) into $SKILLS_DIR
# (default ~/.claude/skills; antigravity sets ~/.gemini/antigravity-cli/skills). Pods
# have no catalog checkout, so this carries the files. A .catalog-managed marker GCs a
# skill that was later disabled without touching skills we don't manage. Non-fatal.
# Canonical source: agents-shared-capabilities templates/pod-apply/. ADR 0002.
set -u
SRC="${MCP_ARTIFACT_DIR:-/etc/openab/mcp}"
B64="$SRC/skills.tar.b64"
LIST="$SRC/skills.list"
DIR="${SKILLS_DIR:-$HOME/.claude/skills}"
MARK="$DIR/.catalog-managed"
[ -f "$B64" ] || { echo "skills-apply: no $B64 — nothing to do"; exit 0; }
mkdir -p "$DIR"

newnames=""
[ -f "$LIST" ] && newnames=$(cat "$LIST")

# GC skills we previously managed but are no longer enabled.
if [ -f "$MARK" ]; then
  while read -r old; do
    [ -n "$old" ] || continue
    printf '%s\n' "$newnames" | grep -qx "$old" && continue
    rm -rf "$DIR/$old"; echo "skills-apply: $old removed (no longer enabled)"
  done < "$MARK"
fi

# Clear the enabled skills' dirs (so removed files don't linger) then re-extract.
printf '%s\n' "$newnames" | while read -r nm; do [ -n "$nm" ] && rm -rf "$DIR/$nm"; done
if [ -s "$B64" ]; then
  base64 -d "$B64" | tar -xf - -C "$DIR"
  echo "skills-apply: projected [$(printf '%s' "$newnames" | tr '\n' ' ')] → $DIR"
fi

# Record the managed set for next run's GC.
if [ -f "$LIST" ]; then cp "$LIST" "$MARK"; else : > "$MARK"; fi
