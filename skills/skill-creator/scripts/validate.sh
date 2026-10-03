#!/bin/sh
# validate.sh — check a skill directory against the Agent Skills spec
# (agentskills.io/specification) plus this catalog's conventions.
#
# Usage: validate.sh [--strict-spec] <skill-dir> [<skill-dir> ...]
#
#   --strict-spec  treat non-spec top-level frontmatter keys (including this
#                  catalog's extension keys) as errors instead of warnings —
#                  use when the skill must pass `skills-ref validate`.
#
# POSIX sh + awk only (no python/node): pods do not ship an interpreter.
# Exit 0 = no errors (warnings allowed), 1 = errors found, 2 = usage error.

set -u

strict=0
case "${1:-}" in
  --strict-spec) strict=1; shift ;;
  -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
esac
[ $# -ge 1 ] || { echo "usage: $0 [--strict-spec] <skill-dir> [...]" >&2; exit 2; }

SPEC_KEYS=" name description license compatibility metadata allowed-tools "
CATALOG_KEYS=" capability source pinned-ref checksum requires "

total_err=0

# Print a top-level scalar from frontmatter text on stdin: handles plain,
# quoted, multi-line plain, and block (| >) scalars. Prints nothing if absent.
get_scalar() {
  awk -v k="$1" '
    function flush() { if (v ~ /^".*"$/ || v ~ /^\047.*\047$/) v = substr(v, 2, length(v) - 2); print v; done = 1 }
    st == 1 {
      if ($0 ~ /^[ \t]/ || $0 == "") {
        l = $0; sub(/^[ \t]+/, "", l)
        if (sep == "\n") v = v (v == "" ? "" : "\n") l
        else if (l != "") v = v (v == "" ? "" : " ") l
        next
      }
      flush(); exit
    }
    st == 0 && index($0, k ":") == 1 {
      v = substr($0, length(k) + 2); sub(/^[ \t]+/, "", v); sub(/[ \t]+$/, "", v)
      if (v ~ /^[|>][-+0-9]*$/) { sep = (substr(v, 1, 1) == "|") ? "\n" : " "; v = "" }
      else sep = " "
      st = 1
    }
    END { if (st == 1 && !done) flush() }'
}

# Unicode character count of stdin (counts non-continuation UTF-8 bytes).
char_count() {
  LC_ALL=C awk '{ s = $0; n += gsub(/[^\200-\277]/, "", s) } NR > 1 { n++ } END { print n + 0 }'
}

validate_one() {
  dir=${1%/}
  err=0
  e() { echo "  ERROR: $*"; err=$((err + 1)); }
  w() { echo "  warn:  $*"; }

  echo "== $dir"
  sk="$dir/SKILL.md"
  if [ ! -f "$sk" ]; then e "SKILL.md not found"; return "$err"; fi

  # --- frontmatter block -------------------------------------------------
  fm=$(awk 'NR == 1 { if ($0 !~ /^---[ \t\r]*$/) { bad = 1; exit 3 } next }
            /^---[ \t\r]*$/ { closed = 1; exit }
            { print }
            END { if (bad) exit 3; if (!closed) exit 4 }' "$sk")
  case $? in
    3) e "SKILL.md must start with a '---' frontmatter line"; return "$err" ;;
    4) e "frontmatter is not closed with '---'"; return "$err" ;;
  esac

  # --- top-level keys ----------------------------------------------------
  ext=""
  for key in $(printf '%s\n' "$fm" | awk -F: '/^[A-Za-z0-9_-]+:/ { print $1 }'); do
    case "$SPEC_KEYS" in *" $key "*) continue ;; esac
    case "$CATALOG_KEYS" in
      *" $key "*) ext="$ext $key"; continue ;;
    esac
    msg="unknown top-level key '$key' (spec allows:$SPEC_KEYS)"
    if [ "$strict" = 1 ]; then e "$msg"; else w "$msg"; fi
  done
  if [ -n "$ext" ]; then
    msg="catalog extension keys not in the Agent Skills spec:$ext (skills-ref validate rejects them)"
    if [ "$strict" = 1 ]; then e "$msg"; else w "$msg"; fi
  fi

  # --- name --------------------------------------------------------------
  name=$(printf '%s\n' "$fm" | get_scalar name | sed 's/[ \t]#.*$//')
  base=$(basename "$dir")
  if [ -z "$name" ]; then
    e "frontmatter 'name' missing or empty"
  else
    nlen=$(printf '%s' "$name" | char_count)
    [ "$nlen" -le 64 ] || e "name is $nlen chars (max 64)"
    printf '%s' "$name" | grep -Eq '^[a-z0-9]+(-[a-z0-9]+)*$' ||
      e "name '$name' must be lowercase a-z/0-9 and single hyphens, not starting/ending with '-'"
    [ "$name" = "$base" ] || e "name '$name' must match directory name '$base'"
    case "$name" in *claude*|*anthropic*) e "name must not contain reserved words 'claude' / 'anthropic'" ;; esac
  fi

  # --- description -------------------------------------------------------
  desc=$(printf '%s\n' "$fm" | get_scalar description)
  if [ -z "$desc" ]; then
    e "frontmatter 'description' missing or empty"
  else
    dlen=$(printf '%s' "$desc" | char_count)
    [ "$dlen" -le 1024 ] || e "description is $dlen chars (max 1024)"
    case "$desc" in
      *TODO*|*\<*\>*) e "description still contains a TODO / <placeholder>" ;;
    esac
    [ "$dlen" -ge 40 ] || w "description is very short ($dlen chars) — say what it does AND when to use it"
  fi

  # --- compatibility -----------------------------------------------------
  comp=$(printf '%s\n' "$fm" | get_scalar compatibility)
  if [ -n "$comp" ]; then
    clen=$(printf '%s' "$comp" | char_count)
    [ "$clen" -le 500 ] || e "compatibility is $clen chars (max 500)"
  fi

  # --- body --------------------------------------------------------------
  lines=$(wc -l < "$sk" | tr -d ' ')
  [ "$lines" -le 500 ] || w "SKILL.md is $lines lines (spec recommends < 500; move detail to references/)"
  body=$(awk 'NR == 1 { next } !c && /^---[ \t\r]*$/ { c = 1; next } c' "$sk")
  printf '%s' "$body" | grep -q '[^[:space:]]' || w "SKILL.md body is empty"
  printf '%s\n' "$body" | grep -n 'TODO' | head -3 | while IFS= read -r l; do
    echo "  warn:  body has TODO (body line ${l%%:*})"
  done

  # --- relative links resolve (outside fenced code) ----------------------
  links=$(printf '%s\n' "$body" | awk '
    /^[ \t]*(```|~~~)/ { fence = !fence; next }
    fence { next }
    { s = $0
      while (match(s, /\]\([^)]+\)/)) {
        t = substr(s, RSTART + 2, RLENGTH - 3); s = substr(s, RSTART + RLENGTH)
        sub(/[ \t]+".*$/, "", t); sub(/#.*$/, "", t)
        if (t == "" || t ~ /^[a-zA-Z][a-zA-Z0-9+.-]*:/ || t ~ /^\// || t ~ /[<>]/) continue
        print t
      } }')
  for t in $links; do
    [ -e "$dir/$t" ] || e "linked file not found: $t"
  done

  # --- scripts executable ------------------------------------------------
  if [ -d "$dir/scripts" ]; then
    for f in "$dir"/scripts/*; do
      [ -f "$f" ] || continue
      [ -x "$f" ] || w "script not executable: ${f#"$dir"/} (chmod +x, and commit the mode)"
    done
  fi

  if [ "$err" -eq 0 ]; then echo "  ok"; fi
  return "$err"
}

for d in "$@"; do
  validate_one "$d"
  total_err=$((total_err + $?))
done

if [ "$total_err" -gt 0 ]; then
  echo "FAIL: $total_err error(s)"
  exit 1
fi
echo "PASS"
