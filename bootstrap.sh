#!/usr/bin/env bash
#
# bootstrap.sh — idempotent capability provisioning for a coding-agent host.
#
# Clones (or fast-forwards) the three canonical repos, lays down the ~/ symlink
# layout, generates ~/personal/capabilities.md from the template on first run,
# runs sync.js to project the enabled subset into each installed runtime, then
# prints per-runtime reload hints. Safe to re-run — never clobbers an existing
# capabilities.md or a non-symlink at a managed path.
#
# ADR: agents-cold-memory:shared/adr/0002-agent-capability-provisioning (Decision 8).
#
# Usage:
#   AGENT_UID=shadow ./bootstrap.sh
#
# Env overrides (all optional except AGENT_UID):
#   AGENT_UID        agent uid; also the cold namespace agent-bot/<uid>   (required)
#   HOME             target home (default: $HOME)
#   CAP_REMOTE       agents-shared-capabilities git URL
#   MEM_REMOTE       agents-shared-memory git URL
#   COLD_REMOTE      agents-cold-memory git URL
#   CAP_DIR MEM_DIR COLD_DIR   local checkout dirs (defaults under $HOME)
#   NO_SYNC=1        skip the sync step (clone + symlink + generate only)
#   NO_PULL=1        do not fast-forward existing checkouts (clone-if-absent only)
set -euo pipefail

: "${AGENT_UID:?set AGENT_UID (the agent uid / cold namespace agent-bot/<uid>)}"

GH="${GH_ORG:-rockexe0000}"
CAP_REMOTE="${CAP_REMOTE:-https://github.com/${GH}/agents-shared-capabilities.git}"
MEM_REMOTE="${MEM_REMOTE:-https://github.com/${GH}/agents-shared-memory.git}"
COLD_REMOTE="${COLD_REMOTE:-https://github.com/${GH}/agents-cold-memory.git}"

CAP_DIR="${CAP_DIR:-$HOME/.agents-shared-capabilities}"
MEM_DIR="${MEM_DIR:-$HOME/.agents-shared-memory}"
COLD_DIR="${COLD_DIR:-$HOME/.agents-cold-memory}"

log() { printf '\033[1m::\033[0m %s\n' "$*"; }

# clone <remote> <dir> — clone if absent, else fast-forward (unless NO_PULL).
clone_or_pull() {
  local remote="$1" dir="$2"
  if [ -d "$dir/.git" ]; then
    if [ "${NO_PULL:-0}" = "1" ]; then
      log "exists (no-pull): $dir"
    else
      log "pull: $dir"
      git -C "$dir" pull --ff-only --quiet || log "  WARN fast-forward failed (local changes?) — left as-is"
    fi
  else
    log "clone: $remote -> $dir"
    git clone --quiet "$remote" "$dir"
  fi
}

# link <target> <linkpath> — idempotent symlink; refuses to replace a real file/dir.
link() {
  local target="$1" linkpath="$2"
  if [ -L "$linkpath" ]; then
    [ "$(readlink "$linkpath")" = "$target" ] || ln -sfn "$target" "$linkpath"
  elif [ -e "$linkpath" ]; then
    log "  SKIP $linkpath — exists and is not a symlink (leaving as-is)"
    return 0
  else
    ln -sfn "$target" "$linkpath"
  fi
  log "  link $linkpath -> $target"
}

log "AGENT_UID=$AGENT_UID  HOME=$HOME"

# 1) three canonical repos
clone_or_pull "$MEM_REMOTE"  "$MEM_DIR"
clone_or_pull "$CAP_REMOTE"  "$CAP_DIR"
clone_or_pull "$COLD_REMOTE" "$COLD_DIR"

# 2) ~/ symlink layout (Hot/Warm sources)
NS_DIR="$COLD_DIR/content/docs/agent-bot/$AGENT_UID"
PERSONAL_DIR="$NS_DIR/personal"
mkdir -p "$PERSONAL_DIR"
link "$MEM_DIR/AGENTS.md" "$HOME/AGENTS.md"
[ -e "$MEM_DIR/CLAUDE.md" ] && link "$MEM_DIR/CLAUDE.md" "$HOME/CLAUDE.md"
link "$MEM_DIR/shared"    "$HOME/shared"
link "$PERSONAL_DIR"      "$HOME/personal"

# 3) generate capabilities.md on first run (never clobber)
CAP_MD="$PERSONAL_DIR/capabilities.md"
if [ -e "$CAP_MD" ]; then
  log "capabilities.md exists — kept"
else
  log "generate capabilities.md from template"
  cp "$CAP_DIR/templates/capabilities.template.md" "$CAP_MD"
  log "  NOTE edit $CAP_MD to enable skills/MCP (defaults: all off), then re-run"
fi

# 4) project into installed runtimes
if [ "${NO_SYNC:-0}" = "1" ]; then
  log "NO_SYNC=1 — skipping sync"
else
  log "sync"
  node "$CAP_DIR/tools/sync.js"
fi

# 5) reload hints (runtime reload semantics differ; do not force-restart a live agent)
cat <<'EOF'

:: reload (per runtime, as applicable):
   - Codex        : restart the process (config.toml read at startup)
   - Claude Code  : restart / reload the session to pick up new MCP servers + skills
   - Antigravity  : restart the CLI
   - opencode     : restart to reload ~/.config/opencode/opencode.json
   MCP behind oab-facade also needs the facade (openab) to have reloaded its mcp.json.

:: bootstrap done.
EOF
