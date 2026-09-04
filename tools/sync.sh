#!/usr/bin/env bash
# thin wrapper — logic lives in sync.js (portable YAML/frontmatter handling in Node).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec node "$DIR/sync.js" "$@"
