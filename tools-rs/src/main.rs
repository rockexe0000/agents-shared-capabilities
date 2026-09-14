//! capsync — ADR 0008 Phase 0 spike + Phase 1a (MCP render axis).
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
//!   - MCP axis (openab-agent-mcp.json / runtime-mcp.json) added in Phase 1a; bin /
//!     skills / hooks / --check / --with-tools remain for later Phase 1 slices.

use std::collections::BTreeSet;
use std::collections::HashMap;
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
            "usage: capsync --render <outdir> [--capabilities <file>] [--catalog <dir>]\n\
                    (emits authz-*.json + openab-agent-mcp.json + runtime-mcp.json)"
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

    // ---- MCP axis (ADR 0008 Phase 1a) — port of sync.js buildArtifacts() MCP path ----
    // registry = catalog ∪ personal (personal overrides on name collision); split the
    // enabled servers by route (default facade), shape each via oab-facade shapeServer,
    // emit { mcpServers: {...} } as JSON.stringify(_, null, 2)+"\n". Render does NOT
    // resolve secrets — both routes go through shapeServer, which rewrites env:VAR ->
    // ${env:VAR}. Catalog root comes from --catalog (see resolve_catalog).
    let catalog = resolve_catalog(args);
    let base = personal_base(&cap_file);
    let registry = build_registry(catalog.as_deref(), base.as_deref());
    let enable = parse_enable(&cap_file);
    let (facade, direct) = split_mcp_routes(&enable, &registry);
    let out = Path::new(&out_dir);
    std::fs::write(out.join("openab-agent-mcp.json"), as_cfg(&facade))
        .map_err(|e| format!("write openab-agent-mcp.json: {e}"))?;
    std::fs::write(out.join("runtime-mcp.json"), as_cfg(&direct))
        .map_err(|e| format!("write runtime-mcp.json: {e}"))?;
    println!(
        "rendered openab-agent-mcp.json (facade: {}) / runtime-mcp.json (direct: {})",
        names(&facade),
        names(&direct),
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

#[derive(Clone)]
enum Json {
    Str(String),
    Num(String), // verbatim token; V8 number canonicalization is not reproduced (unused by MCP args)
    Bool(bool),
    Null,
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
        Json::Num(n) => n.clone(),
        Json::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
        Json::Null => "null".to_string(),
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

// ============================================================================
// MCP axis (ADR 0008 Phase 1a) — port of sync.js buildArtifacts() MCP path:
// parseEnable + parseRegistry + buildRegistry + oab-facade shapeServer.
// ============================================================================

const DEFAULT_ROUTE: &str = "facade"; // policy: MCP hides behind oab-facade unless route: direct

/// Catalog (REPO) root: `--catalog <dir>` if given, else inferred from the exe path
/// (<root>/tools-rs/target/<profile>/capsync). sync.js gets REPO free via __dirname; a
/// standalone binary can't, so Phase 1 takes it as an explicit flag (see ADR 0008 handoff).
fn resolve_catalog(args: &[String]) -> Option<PathBuf> {
    if let Some(v) = flag_value(args, "--catalog").filter(|v| !v.starts_with("--")) {
        return Some(PathBuf::from(v));
    }
    let exe = std::env::current_exe().ok()?;
    // capsync -> <profile> -> target -> tools-rs -> repo root
    exe.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
}

/// JS: base = path.dirname(path.dirname(fs.realpathSync(capFile))) — the agent's personal
/// namespace root (…/agent-bot/{uid}). None if the path can't be resolved.
fn personal_base(cap_file: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(cap_file).ok()?;
    real.parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
}

/// A registry MCP server, holding only the fields the render output depends on.
/// env/headers are ordered Vecs to preserve the file's insertion order (byte parity).
#[derive(Clone, Default)]
struct Server {
    name: String,
    route: Option<String>,
    transport: Option<String>,
    url: Option<String>,
    command: Option<String>,
    args: Option<Json>,
    env: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

fn load_mcp_registry(dir: &Path) -> Vec<Server> {
    let f = dir.join("mcp").join("registry.yaml");
    match std::fs::read_to_string(&f) {
        Ok(text) => parse_registry(&text),
        Err(_) => Vec::new(),
    }
}

/// JS buildRegistry: catalog then personal (personal overrides on name collision).
/// Map order is irrelevant — render output order follows the enable-list, not the map.
fn build_registry(catalog: Option<&Path>, base: Option<&Path>) -> HashMap<String, Server> {
    let mut map: HashMap<String, Server> = HashMap::new();
    if let Some(c) = catalog {
        for s in load_mcp_registry(c) {
            map.insert(s.name.clone(), s);
        }
    }
    if let Some(b) = base {
        for s in load_mcp_registry(b) {
            map.insert(s.name.clone(), s);
        }
    }
    map
}

/// Port of parse.js parseRegistry for the fields render needs. 2-space item indent, 4-space
/// props, `env:`/`headers:` open a nested section at indent 4 whose keys sit at indent >= 6.
fn parse_registry(text: &str) -> Vec<Server> {
    let mut servers: Vec<Server> = Vec::new();
    let mut section: Option<String> = None; // "env" | "headers" | "capability" | ...
    for raw in text.split('\n') {
        let line = trim_end(raw);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= 2 {
            if let Some(name) = match_item_name(t) {
                servers.push(Server {
                    name: strip(&name),
                    ..Default::default()
                });
                section = None;
                continue;
            }
        }
        let cur = match servers.last_mut() {
            Some(c) => c,
            None => continue,
        };
        if indent == 4 {
            if let Some(k) = match_opener(t) {
                section = Some(k);
                continue;
            }
        }
        let (key, val) = match match_kv(t) {
            Some(kv) => kv,
            None => continue,
        };
        if let Some(sec) = &section {
            if indent >= 6 {
                if sec == "env" {
                    upsert(&mut cur.env, &key, strip(&val));
                } else if sec == "headers" {
                    upsert(&mut cur.headers, &key, strip(&val));
                }
                continue;
            }
        }
        section = None;
        match key.as_str() {
            "args" => cur.args = Some(parse_args(&val)),
            "route" => cur.route = Some(strip(&val)),
            "transport" => cur.transport = Some(strip(&val)),
            "url" => cur.url = Some(strip(&val)),
            "command" => cur.command = Some(strip(&val)),
            _ => {} // other scalars (source, pinned-ref, auth, …) don't affect render output
        }
    }
    servers
}

/// JS: t.match(/^-\s*name:\s*(.+)$/) group 1 (on an already-trimmed line).
fn match_item_name(t: &str) -> Option<String> {
    let r = t.strip_prefix('-')?;
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix("name:")?;
    let r = r.trim_start_matches([' ', '\t']);
    if r.is_empty() {
        None
    } else {
        Some(r.to_string())
    }
}

/// JS: t.match(/^([\w-]+):\s*$/) group 1 (section opener; `t` trimmed so nothing follows `:`).
fn match_opener(t: &str) -> Option<String> {
    let k = t.strip_suffix(':')?;
    if k.is_empty()
        || !k
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(k.to_string())
}

/// JS: t.match(/^([\w-]+):\s*(.*)$/) -> (key, value). Value may be empty.
fn match_kv(t: &str) -> Option<(String, String)> {
    let idx = t.find(':')?;
    let key = &t[..idx];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let rest = t[idx + 1..].trim_start_matches([' ', '\t']);
    Some((key.to_string(), rest.to_string()))
}

/// Insertion-order-preserving map set: update in place on key collision, else append.
fn upsert(v: &mut Vec<(String, String)>, k: &str, val: String) {
    if let Some(e) = v.iter_mut().find(|(ek, _)| ek == k) {
        e.1 = val;
    } else {
        v.push((k.to_string(), val));
    }
}

/// JS: JSON.parse(stripComment(value)); on throw -> []. MCP args are JSON string arrays in
/// practice (numbers, if ever present, are echoed verbatim — see Json::Num).
fn parse_args(val: &str) -> Json {
    let s = strip_comment(val);
    parse_json(s.trim()).unwrap_or_else(|_| Json::Arr(Vec::new()))
}

/// Enabled MCP set from capabilities.md (Phase 1a needs only the MCP axis; skills/hooks
/// land with their own slices). Mirrors parse.js parseEnable's `mcp.servers` handling.
#[derive(Default)]
struct Enable {
    mcp: Vec<EnableMcp>,
}
struct EnableMcp {
    name: String,
}

fn parse_enable(file: &Path) -> Enable {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(_) => return Enable::default(),
    };
    let body = strip_frontmatter(&text);
    let mut out = Enable::default();
    let mut top: Option<String> = None;
    let mut mcp_servers = false;
    for raw in body.split('\n') {
        let tl = raw.trim();
        if tl.is_empty() || tl.starts_with('#') {
            continue;
        }
        if let Some(k) = match_top_key(raw) {
            top = Some(k);
            mcp_servers = false;
            continue;
        }
        if top.as_deref() == Some("mcp") {
            if is_servers_line(raw) {
                mcp_servers = true;
                continue;
            }
            if mcp_servers {
                if let Some(name) = match_dash_name(raw) {
                    out.mcp.push(EnableMcp { name });
                }
            }
        }
    }
    out
}

/// JS: raw.match(/^(\w[\w-]*):\s*$/) group 1 — a top-level key at column 0 (no indent).
fn match_top_key(raw: &str) -> Option<String> {
    let idx = raw.find(':')?;
    let key = &raw[..idx];
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    if raw[idx + 1..].chars().all(|c| c.is_whitespace()) {
        Some(key.to_string())
    } else {
        None
    }
}

/// JS: /^\s*servers:\s*$/.test(raw).
fn is_servers_line(raw: &str) -> bool {
    raw.trim() == "servers:"
}

/// JS: raw.match(/^\s*-\s*\{?\s*name:\s*([A-Za-z0-9_-]+)/) group 1 (block or inline item).
fn match_dash_name(raw: &str) -> Option<String> {
    let r = raw.trim_start();
    let r = r.strip_prefix('-')?;
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix('{').unwrap_or(r);
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix("name:")?;
    let r = r.trim_start_matches([' ', '\t']);
    let name: String = r
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// JS: toRef — env:VAR -> ${env:VAR} (openab interpolation); literals pass through.
/// /^env:(.+)$/ requires >= 1 char after the prefix.
fn to_ref(v: &str) -> String {
    if let Some(rest) = v.strip_prefix("env:").filter(|r| !r.is_empty()) {
        format!("${{env:{rest}}}")
    } else {
        v.to_string()
    }
}

fn map_vals(pairs: &[(String, String)]) -> Json {
    Json::Obj(
        pairs
            .iter()
            .map(|(k, v)| (k.clone(), Json::Str(to_ref(v))))
            .collect(),
    )
}

/// Port of oab-facade shapeServer: http -> {type,url,[headers]}; else stdio ->
/// {type,[command],args,env}. Fields absent in the registry are omitted (JS undefined).
fn shape_server(s: &Server) -> Json {
    if s.transport.as_deref() == Some("http") {
        let mut e: Vec<(String, Json)> = vec![("type".to_string(), Json::Str("http".to_string()))];
        if let Some(u) = &s.url {
            e.push(("url".to_string(), Json::Str(u.clone())));
        }
        if !s.headers.is_empty() {
            e.push(("headers".to_string(), map_vals(&s.headers)));
        }
        Json::Obj(e)
    } else {
        let mut e: Vec<(String, Json)> = vec![("type".to_string(), Json::Str("stdio".to_string()))];
        if let Some(cmd) = &s.command {
            e.push(("command".to_string(), Json::Str(cmd.clone())));
        }
        e.push((
            "args".to_string(),
            s.args.clone().unwrap_or_else(|| Json::Arr(Vec::new())),
        ));
        e.push(("env".to_string(), map_vals(&s.env)));
        Json::Obj(e)
    }
}

/// JS: JSON.stringify({ mcpServers: {name: shape(s)} }, null, 2) + "\n".
fn as_cfg(list: &[Server]) -> String {
    let servers: Vec<(String, Json)> = list
        .iter()
        .map(|s| (s.name.clone(), shape_server(s)))
        .collect();
    let obj = Json::Obj(vec![("mcpServers".to_string(), Json::Obj(servers))]);
    let mut out = stringify(&obj, 0);
    out.push('\n');
    out
}

fn names(list: &[Server]) -> String {
    if list.is_empty() {
        "none".to_string()
    } else {
        list.iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Split the enabled servers into (facade, direct) by route, preserving enable-list order.
/// Missing names are skipped with a stderr note (JS console.error) — not part of parity.
fn split_mcp_routes(enable: &Enable, reg: &HashMap<String, Server>) -> (Vec<Server>, Vec<Server>) {
    let mut facade = Vec::new();
    let mut direct = Vec::new();
    for want in &enable.mcp {
        match reg.get(&want.name) {
            None => eprintln!("  {}: not in registry — skipped", want.name),
            Some(def) => {
                let route = def.route.as_deref().unwrap_or(DEFAULT_ROUTE);
                if route == "direct" {
                    direct.push(def.clone());
                } else {
                    facade.push(def.clone());
                }
            }
        }
    }
    (facade, direct)
}

// ----------------------------------------------------------------------------
// Minimal JSON value parser — only for a registry server's inline `args:` array, so it can
// be re-emitted via the shared printer (JSON.parse then JSON.stringify is not identity on
// formatting). Numbers are kept as their source token (see Json::Num).
// ----------------------------------------------------------------------------

fn parse_json(s: &str) -> Result<Json, ()> {
    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0usize;
    skip_ws(&chars, &mut pos);
    let v = parse_value(&chars, &mut pos)?;
    skip_ws(&chars, &mut pos);
    if pos != chars.len() {
        return Err(());
    }
    Ok(v)
}

fn skip_ws(c: &[char], pos: &mut usize) {
    while *pos < c.len() && matches!(c[*pos], ' ' | '\t' | '\n' | '\r') {
        *pos += 1;
    }
}

fn parse_value(c: &[char], pos: &mut usize) -> Result<Json, ()> {
    skip_ws(c, pos);
    match c.get(*pos) {
        Some('"') => parse_json_string(c, pos).map(Json::Str),
        Some('[') => parse_array(c, pos),
        Some('{') => parse_object(c, pos),
        Some('t') => parse_lit(c, pos, "true", Json::Bool(true)),
        Some('f') => parse_lit(c, pos, "false", Json::Bool(false)),
        Some('n') => parse_lit(c, pos, "null", Json::Null),
        Some(ch) if *ch == '-' || ch.is_ascii_digit() => parse_number(c, pos),
        _ => Err(()),
    }
}

fn parse_lit(c: &[char], pos: &mut usize, lit: &str, val: Json) -> Result<Json, ()> {
    for lc in lit.chars() {
        if c.get(*pos) != Some(&lc) {
            return Err(());
        }
        *pos += 1;
    }
    Ok(val)
}

fn parse_json_string(c: &[char], pos: &mut usize) -> Result<String, ()> {
    if c.get(*pos) != Some(&'"') {
        return Err(());
    }
    *pos += 1;
    let mut out = String::new();
    while let Some(&ch) = c.get(*pos) {
        *pos += 1;
        match ch {
            '"' => return Ok(out),
            '\\' => {
                let esc = *c.get(*pos).ok_or(())?;
                *pos += 1;
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000C}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let h = *c.get(*pos).ok_or(())?;
                            *pos += 1;
                            code = code * 16 + h.to_digit(16).ok_or(())?;
                        }
                        out.push(char::from_u32(code).ok_or(())?);
                    }
                    _ => return Err(()),
                }
            }
            _ => out.push(ch),
        }
    }
    Err(())
}

fn parse_number(c: &[char], pos: &mut usize) -> Result<Json, ()> {
    let start = *pos;
    if c.get(*pos) == Some(&'-') {
        *pos += 1;
    }
    while let Some(&ch) = c.get(*pos) {
        if ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E' || ch == '+' || ch == '-' {
            *pos += 1;
        } else {
            break;
        }
    }
    if *pos == start {
        return Err(());
    }
    let tok: String = c[start..*pos].iter().collect();
    Ok(Json::Num(tok))
}

fn parse_array(c: &[char], pos: &mut usize) -> Result<Json, ()> {
    *pos += 1; // consume '['
    let mut items = Vec::new();
    skip_ws(c, pos);
    if c.get(*pos) == Some(&']') {
        *pos += 1;
        return Ok(Json::Arr(items));
    }
    loop {
        items.push(parse_value(c, pos)?);
        skip_ws(c, pos);
        match c.get(*pos) {
            Some(',') => *pos += 1,
            Some(']') => {
                *pos += 1;
                return Ok(Json::Arr(items));
            }
            _ => return Err(()),
        }
    }
}

fn parse_object(c: &[char], pos: &mut usize) -> Result<Json, ()> {
    *pos += 1; // consume '{'
    let mut entries: Vec<(String, Json)> = Vec::new();
    skip_ws(c, pos);
    if c.get(*pos) == Some(&'}') {
        *pos += 1;
        return Ok(Json::Obj(entries));
    }
    loop {
        skip_ws(c, pos);
        let key = parse_json_string(c, pos)?;
        skip_ws(c, pos);
        if c.get(*pos) != Some(&':') {
            return Err(());
        }
        *pos += 1;
        let v = parse_value(c, pos)?;
        entries.push((key, v));
        skip_ws(c, pos);
        match c.get(*pos) {
            Some(',') => *pos += 1,
            Some('}') => {
                *pos += 1;
                return Ok(Json::Obj(entries));
            }
            _ => return Err(()),
        }
    }
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

    const MCP_REG: &str = "\
servers:
  - name: test-stdio
    transport: stdio
    command: npx
    args: [\"-y\", \"@example/mcp-server\"]
    env:
      API_KEY: \"env:EXAMPLE_API_KEY\"
      REGION: \"us-east-1\"
    capability:
      description: \"ignored\"
    source: \"fixture\"
  - name: test-http
    route: direct
    transport: http
    url: \"https://mcp.example.com/v1\"
    headers:
      Authorization: \"env:EXAMPLE_TOKEN\"
";

    fn registry_of(text: &str) -> HashMap<String, Server> {
        let mut m = HashMap::new();
        for s in parse_registry(text) {
            m.insert(s.name.clone(), s);
        }
        m
    }

    #[test]
    fn mcp_route_split_and_shape() {
        let reg = registry_of(MCP_REG);
        assert_eq!(reg.len(), 2);
        let enable = Enable {
            mcp: vec![
                EnableMcp {
                    name: "test-stdio".into(),
                },
                EnableMcp {
                    name: "test-http".into(),
                },
            ],
        };
        let (facade, direct) = split_mcp_routes(&enable, &reg);
        // default route (facade) for the stdio server, explicit direct for the http one.
        assert_eq!(facade.len(), 1);
        assert_eq!(facade[0].name, "test-stdio");
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].name, "test-http");

        // http shape is short enough to assert byte-for-byte; env refs -> ${env:} rewrite.
        assert_eq!(
            as_cfg(&direct),
            "{\n  \"mcpServers\": {\n    \"test-http\": {\n      \"type\": \"http\",\n      \"url\": \"https://mcp.example.com/v1\",\n      \"headers\": {\n        \"Authorization\": \"${env:EXAMPLE_TOKEN}\"\n      }\n    }\n  }\n}\n"
        );
        // stdio shape: spot-check the tricky bits (env rewrite kept + literal passthrough + args).
        let stdio = as_cfg(&facade);
        assert!(stdio.contains("\"type\": \"stdio\""));
        assert!(stdio.contains("\"command\": \"npx\""));
        assert!(stdio
            .contains("\"args\": [\n        \"-y\",\n        \"@example/mcp-server\"\n      ]"));
        assert!(stdio.contains("\"API_KEY\": \"${env:EXAMPLE_API_KEY}\""));
        assert!(stdio.contains("\"REGION\": \"us-east-1\"")); // non-env literal untouched
    }

    #[test]
    fn mcp_empty_cfg() {
        assert_eq!(as_cfg(&[]), "{\n  \"mcpServers\": {}\n}\n");
    }

    #[test]
    fn enable_matchers() {
        assert_eq!(match_top_key("mcp:"), Some("mcp".to_string()));
        assert_eq!(match_top_key("  - name: x"), None); // indented -> not a top key
        assert!(is_servers_line("  servers:"));
        assert!(!is_servers_line("servers: x"));
        assert_eq!(
            match_dash_name("    - name: alpha"),
            Some("alpha".to_string())
        );
        assert_eq!(
            match_dash_name("    - { name: beta }"),
            Some("beta".to_string())
        );
    }

    #[test]
    fn json_args_reformat() {
        // JSON.parse then stringify(_, null, 2) reindents each element on its own line.
        let j = parse_json("[\"-y\", \"x\"]").unwrap();
        assert_eq!(stringify(&j, 0), "[\n  \"-y\",\n  \"x\"\n]");
        assert!(parse_json("nope").is_err());
    }
}
