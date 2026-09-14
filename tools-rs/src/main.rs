//! capsync — ADR 0008 Phase 0 spike.
//!
//! A single-file, runtime-portable reimplementation of the capability tooling's
//! `--render`, scoped for this spike to the **authorization axis** (閘 2, ADR 0007):
//!
//!   parse permissions.md  →  runtime-agnostic {allow, deny}  →  per-runtime tokens
//!   →  emit authz-<runtime>.json
//!
//! Goal: **byte-for-byte parity** with `tools/sync.js --render`'s
//! `authz-antigravity.json` and `authz-claude-code.json`. Every behaviour below is a
//! deliberate mirror of `tools/lib/parse.js` (`parsePermissions`, `strip`,
//! `stripComment`) and `tools/lib/authz.js` (`buildAuthz`, `mapAuthz`), including the
//! JSON is emitted as `JSON.stringify(obj, null, 2) + "\n"`.
//!
//! SPIKE SCOPE (intentionally NOT full parity — see the ADR 0008 handoff Phase 1):
//!   - Only the two authz JSON files. `authz-suggest.txt` and `authorize_skill_requires:
//!     auto` derive allows from enabled skills' `requires`, which needs the enable-list +
//!     bin-tools registry (loadBinTools / requiredToolNames). That surface lands in
//!     Phase 1; here `auto` is parsed and honored for the flag, but no suggestions are
//!     synthesized (documented gap, asserted by the parity harness which only exercises
//!     explicit-mode fixtures for the JSON files).
//!   - Other axes (mcp / bin / skills / hooks) are out of scope for the spike.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if !args.iter().any(|a| a == "--render") {
        return Err(
            "usage: capsync --render <outdir> [--capabilities <capabilities.md>]\n\
                    (Phase 0 spike: emits the authz axis only)"
                .into(),
        );
    }
    let out_dir = flag_value(args, "--render")
        .filter(|v| !v.starts_with("--"))
        .ok_or("usage: capsync --render <outdir> [--capabilities <capabilities.md>]")?;

    // Mirror sync.js capFileArg(): default $HOME/personal/capabilities.md.
    let cap_file = match flag_value(args, "--capabilities") {
        Some(v) => PathBuf::from(v),
        None => {
            let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
            Path::new(&home).join("personal").join("capabilities.md")
        }
    };

    // Mirror buildArtifacts(): permissions.md sits beside the capabilities file.
    let perms_file = cap_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("permissions.md");
    let perms_text = std::fs::read_to_string(&perms_file).unwrap_or_default();

    let authz = build_authz(&perms_text);

    std::fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {out_dir}: {e}"))?;
    for rt in AUTHZ_RUNTIMES {
        let mapped = map_authz(rt, &authz);
        let text = stringify_authz(&mapped);
        let path = Path::new(&out_dir).join(format!("authz-{}.json", rt.id));
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    println!(
        "rendered authz-*.json (mode {}; allow: {}; deny: {}) -> {}",
        authz.flag,
        if authz.allow.is_empty() {
            "none".into()
        } else {
            authz.allow.join(", ")
        },
        if authz.deny.is_empty() {
            "none".into()
        } else {
            authz.deny.join(", ")
        },
        out_dir,
    );
    Ok(())
}

/// `--flag value` lookup mirroring the argv scans in sync.js.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

// ----------------------------------------------------------------------------
// permissions.md parsing — port of tools/lib/parse.js parsePermissions/strip.
// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Perms {
    flag: String, // "auto" | "explicit"
    allow: Vec<String>,
    deny: Vec<String>,
    ask: Vec<String>,
}

const CMD_OPERATION: &str = "run command";

/// Port of parsePermissions(text). Returns command names by effect bucket.
fn parse_permissions(text: &str) -> Perms {
    let body = strip_frontmatter(text);
    let mut out = Perms {
        flag: "explicit".into(),
        ..Default::default()
    };
    let mut in_rules = false;
    // current rule: (effect, operation_raw, scope)
    let mut cur: Option<(String, Option<String>, Option<String>)> = None;

    // flush mirrors the JS closure: only `run command` rules with a scope land in a bucket.
    fn flush(cur: &mut Option<(String, Option<String>, Option<String>)>, out: &mut Perms) {
        if let Some((effect, operation, scope)) = cur.take() {
            let op_ok = operation.as_deref().map(strip).as_deref() == Some(CMD_OPERATION);
            if !effect.is_empty() && op_ok {
                if let Some(scope) = scope {
                    let bucket = match effect.as_str() {
                        "allow" => Some(&mut out.allow),
                        "deny" => Some(&mut out.deny),
                        "ask" => Some(&mut out.ask),
                        _ => None, // other effects have no array bucket (JS: out[effect] undefined)
                    };
                    if let Some(b) = bucket {
                        if !b.contains(&scope) {
                            b.push(scope);
                        }
                    }
                }
            }
        }
    }

    for raw in body.split('\n') {
        let line = trim_end(raw); // JS: raw.replace(/\s+$/, '')
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // top-level flag (only before `rules:`)
        if !in_rules {
            if let Some(v) = match_flag(t) {
                out.flag = v;
                continue;
            }
        }
        if t == "rules:" || (t.starts_with("rules:") && t[6..].trim().is_empty()) {
            flush(&mut cur, &mut out);
            in_rules = true;
            continue;
        }
        if !in_rules {
            continue;
        }
        if let Some(rest) = match_prefix(t, "-", "effect:") {
            flush(&mut cur, &mut out);
            cur = Some((strip(&rest), None, None));
            continue;
        }
        if cur.is_none() {
            continue;
        }
        if let Some(rest) = after_key(t, "operation:") {
            cur.as_mut().unwrap().1 = Some(rest.to_string()); // stored raw, stripped in flush
            continue;
        }
        if let Some(rest) = after_key(t, "scope:") {
            cur.as_mut().unwrap().2 = Some(strip(&rest));
            continue;
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// JS: body.replace(/^---\n[\s\S]*?\n---/, ''). Only strips a leading frontmatter block,
/// non-greedily to the FIRST `\n---`. Returns the remainder (which keeps its leading `\n`).
fn strip_frontmatter(text: &str) -> String {
    if let Some(after) = text.strip_prefix("---\n") {
        if let Some(pos) = after.find("\n---") {
            // removed match = "---\n" + after[..pos] + "\n---"; remainder = after[pos+4..]
            return after[pos + 4..].to_string();
        }
    }
    text.to_string()
}

/// JS: t.match(/^authorize_skill_requires:\s*(auto|explicit)\b/) -> group 1.
fn match_flag(t: &str) -> Option<String> {
    let rest = t.strip_prefix("authorize_skill_requires:")?;
    let rest = rest.trim_start_matches([' ', '\t']);
    for kw in ["auto", "explicit"] {
        if let Some(after) = rest.strip_prefix(kw) {
            // \b: next char must be a non-word char (or end). Word = [A-Za-z0-9_].
            let ok = after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            if ok {
                return Some(kw.to_string());
            }
        }
    }
    None
}

/// JS: t.match(/^-\s*effect:\s*(.+)$/) -> group 1. `t` is already trimmed.
/// Matches a leading `-`, optional ws, `effect:`, optional ws, then a non-empty rest.
fn match_prefix(t: &str, dash: &str, key: &str) -> Option<String> {
    let rest = t.strip_prefix(dash)?;
    let rest = rest.trim_start_matches([' ', '\t']);
    after_key(rest, key)
}

/// JS: /^<key>\s*(.+)$/ on an already-trimmed line -> the `.+` (non-empty) capture.
/// Note `\s*` after the key: leading ws before the value is consumed.
fn after_key(t: &str, key: &str) -> Option<String> {
    let rest = t.strip_prefix(key)?;
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        None // `.+` requires at least one char
    } else {
        Some(rest.to_string())
    }
}

/// JS: strip = s => stripComment(s).trim().replace(/^["']|["']$/g, '')
/// Removes an inline comment, trims, then strips at most one leading and one trailing quote.
fn strip(s: &str) -> String {
    let no_comment = strip_comment(s);
    let trimmed = no_comment.trim();
    let bytes = trimmed.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();
    if end > start && (bytes[start] == b'"' || bytes[start] == b'\'') {
        start += 1;
    }
    if end > start && (bytes[end - 1] == b'"' || bytes[end - 1] == b'\'') {
        end -= 1;
    }
    trimmed[start..end].to_string()
}

/// JS: stripComment — a `#` starts a comment only when outside quotes and preceded by
/// start-of-string, space, or tab. Quote chars `"` and `'` toggle quote state.
fn strip_comment(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            continue;
        }
        if c == '#' && (i == 0 || chars[i - 1] == ' ' || chars[i - 1] == '\t') {
            return chars[..i].iter().collect();
        }
    }
    s.to_string()
}

/// JS: raw.replace(/\s+$/, ''). Trims trailing ASCII/Unicode whitespace.
fn trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| c.is_whitespace())
}

// ----------------------------------------------------------------------------
// authz build/map — port of tools/lib/authz.js buildAuthz/mapAuthz.
// ----------------------------------------------------------------------------

struct Authz {
    flag: String,
    allow: Vec<String>, // sorted, deny-subtracted
    deny: Vec<String>,  // sorted
}

/// Port of buildAuthz for the spike's explicit-mode scope. In `auto` mode the JS adds
/// `requires`-derived commands to allow; that derivation (enable-list + bin tools) is
/// Phase 1 (see module docs), so here allow/deny come solely from permissions.md.
fn build_authz(perms_text: &str) -> Authz {
    let perms = parse_permissions(perms_text);
    let deny: BTreeSet<String> = js_sorted_set(&perms.deny);
    // allow = set(perms.allow) minus deny, then JS-sorted.
    let allow_set: BTreeSet<String> = perms
        .allow
        .iter()
        .filter(|c| !deny.contains(*c))
        .cloned()
        .collect();
    Authz {
        flag: perms.flag,
        allow: js_sort(allow_set.into_iter().collect()),
        deny: js_sort(deny.into_iter().collect()),
    }
}

struct Runtime {
    id: &'static str,
    wrap: fn(&str) -> String,
}

// RUNTIME_AUTHZ: abstract "allow command X" -> runtime-native token.
const AUTHZ_RUNTIMES: &[Runtime] = &[
    Runtime {
        id: "antigravity",
        wrap: wrap_antigravity,
    },
    Runtime {
        id: "claude-code",
        wrap: wrap_claude,
    },
];

fn wrap_antigravity(cmd: &str) -> String {
    format!("command({cmd})")
}
fn wrap_claude(cmd: &str) -> String {
    format!("Bash({cmd}:*)")
}

struct Mapped {
    allow: Vec<String>,
    deny: Vec<String>,
}

/// Port of mapAuthz: apply one runtime's token wrapper to allow/deny (order preserved).
fn map_authz(rt: &Runtime, a: &Authz) -> Mapped {
    Mapped {
        allow: a.allow.iter().map(|c| (rt.wrap)(c)).collect(),
        deny: a.deny.iter().map(|c| (rt.wrap)(c)).collect(),
    }
}

/// JS Array.prototype.sort() default order: by UTF-16 code units. For the ASCII command
/// tokens in play this equals byte order, but we compare by UTF-16 units to be exact.
fn js_sort(mut v: Vec<String>) -> Vec<String> {
    v.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    v
}

/// Dedup helper preserving the JS "Set then sort" semantics via a BTreeSet is not
/// order-safe for UTF-16, so we only use BTreeSet for membership; final order comes from
/// js_sort. This just builds the membership set.
fn js_sorted_set(v: &[String]) -> BTreeSet<String> {
    v.iter().cloned().collect()
}

// ----------------------------------------------------------------------------
// JSON serialization — mirrors JSON.stringify(value, null, 2) + "\n".
// ----------------------------------------------------------------------------

/// Emit `{ "allow": [...], "deny": [...] }` exactly as JSON.stringify(obj, null, 2)+"\n".
fn stringify_authz(m: &Mapped) -> String {
    let obj = Json::Obj(vec![
        ("allow".into(), Json::arr(&m.allow)),
        ("deny".into(), Json::arr(&m.deny)),
    ]);
    let mut s = stringify(&obj, 0);
    s.push('\n');
    s
}

enum Json {
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn arr(items: &[String]) -> Json {
        Json::Arr(items.iter().map(|s| Json::Str(s.clone())).collect())
    }
}

/// Pretty-print with a 2-space step, matching V8's JSON.stringify(x, null, 2):
/// empty [] / {} stay inline; non-empty containers put each element on its own line
/// indented (level+1)*2 spaces, closing bracket at level*2.
fn stringify(j: &Json, level: usize) -> String {
    match j {
        Json::Str(s) => escape_json_string(s),
        Json::Arr(items) => {
            if items.is_empty() {
                return "[]".to_string();
            }
            let inner = " ".repeat((level + 1) * 2);
            let outer = " ".repeat(level * 2);
            let body: Vec<String> = items
                .iter()
                .map(|it| format!("{inner}{}", stringify(it, level + 1)))
                .collect();
            format!("[\n{}\n{outer}]", body.join(",\n"))
        }
        Json::Obj(entries) => {
            if entries.is_empty() {
                return "{}".to_string();
            }
            let inner = " ".repeat((level + 1) * 2);
            let outer = " ".repeat(level * 2);
            let body: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{inner}{}: {}",
                        escape_json_string(k),
                        stringify(v, level + 1)
                    )
                })
                .collect();
            format!("{{\n{}\n{outer}}}", body.join(",\n"))
        }
    }
}

/// Mirror JSON.stringify string escaping: `"` `\` and C0 control chars, with the short
/// escapes \b \t \n \f \r and \u00XX (lowercase) for the rest below 0x20.
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{0009}' => out.push_str("\\t"),
            '\u{000A}' => out.push_str("\\n"),
            '\u{000C}' => out.push_str("\\f"),
            '\u{000D}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_stripped_leading_only() {
        assert_eq!(strip_frontmatter("---\na: 1\n---\nbody\n"), "\nbody\n");
        // no leading frontmatter -> unchanged
        assert_eq!(strip_frontmatter("body\n---\n"), "body\n---\n");
        // non-greedy to first \n---
        assert_eq!(strip_frontmatter("---\nx\n---\ny\n---\n"), "\ny\n---\n");
    }

    #[test]
    fn strip_quotes_and_comments() {
        assert_eq!(strip("\"run command\""), "run command");
        assert_eq!(strip("'cfdrop'"), "cfdrop");
        assert_eq!(strip("cfdrop  # a comment"), "cfdrop");
        assert_eq!(strip("\"a # b\""), "a # b"); // # inside quotes is not a comment
        assert_eq!(strip("bare"), "bare");
    }

    #[test]
    fn parse_explicit_allow_and_deny_override() {
        let text = "\
authorize_skill_requires: explicit
rules:
  - effect: \"allow\"
    operation: \"run command\"
    scope: \"cfdrop\"
  - effect: \"allow\"
    operation: \"run command\"
    scope: \"ripgrep\"
  - effect: \"deny\"
    operation: \"run command\"
    scope: \"ripgrep\"
  - effect: \"ask\"
    operation: \"run command\"
    scope: \"curl\"
  - effect: \"allow\"
    operation: \"push branches + open PRs\"
    scope: \"repo x\"
";
        let a = build_authz(text);
        assert_eq!(a.flag, "explicit");
        assert_eq!(a.allow, vec!["cfdrop"]); // ripgrep removed by deny; push rule ignored
        assert_eq!(a.deny, vec!["ripgrep"]);
    }

    #[test]
    fn dedup_and_sort() {
        let text = "\
rules:
  - effect: allow
    operation: run command
    scope: zebra
  - effect: allow
    operation: run command
    scope: alpha
  - effect: allow
    operation: run command
    scope: alpha
";
        let a = build_authz(text);
        assert_eq!(a.allow, vec!["alpha", "zebra"]);
        assert_eq!(a.flag, "explicit"); // default when flag line absent
    }

    #[test]
    fn json_shape_matches_stringify() {
        let m = Mapped {
            allow: vec!["command(cfdrop)".into()],
            deny: vec![],
        };
        assert_eq!(
            stringify_authz(&m),
            "{\n  \"allow\": [\n    \"command(cfdrop)\"\n  ],\n  \"deny\": []\n}\n"
        );
    }

    #[test]
    fn map_wraps_per_runtime() {
        let a = Authz {
            flag: "explicit".into(),
            allow: vec!["cfdrop".into()],
            deny: vec![],
        };
        let agy = map_authz(&AUTHZ_RUNTIMES[0], &a);
        let cc = map_authz(&AUTHZ_RUNTIMES[1], &a);
        assert_eq!(agy.allow, vec!["command(cfdrop)"]);
        assert_eq!(cc.allow, vec!["Bash(cfdrop:*)"]);
    }
}
