//! capsync — ADR 0008 `--render` port (Phase 0 authz + 1a MCP + 1b bin + 1c skills).
//!
//! A single-file, runtime-portable reimplementation of the capability tooling's `--render`.
//! Goal: **byte-for-byte parity** with `tools/sync.js --render`, mirroring `tools/lib/parse.js`,
//! `tools/lib/authz.js`, `tools/lib/install-bin.js`, and `tools/projectors/oab-facade.js`.
//! All JSON is emitted as `JSON.stringify(obj, null, 2) + "\n"`.
//!
//! Axes covered so far (parity-gated by the tests + parity.sh three-way diff):
//!   - authz (ADR 0007): permissions.md → {allow, deny} per runtime → authz-<runtime>.json,
//!     plus authz-suggest.txt and `authorize_skill_requires: auto` (requires-derived allows).
//!   - MCP (Phase 1a): openab-agent-mcp.json (facade) + runtime-mcp.json (direct).
//!   - bin (Phase 1b): bin-install.tsv from enabled skills' `requires` (∪ --pipeline-bin).
//!   - skills (Phase 1c): skills.tar.b64 (deterministic tar) + skills.list.
//!
//! `--render` (write) and `--check` (re-render + diff committed, exit 1 on drift) are ported,
//! plus `sync` live projection — SKILLS symlink (1e-1), MCP facade + direct for all four
//! runtimes incl. codex TOML with the secret resolver (1e-2), and hooks (1e-3): full parity
//! with sync.js's live path. `--check-tools` (bin drift check, no network) is ported (1f-1);
//! `--with-tools` (install: fetch + verify + extract) lands in 1f-2.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // sync.js dispatch: --render takes precedence, then --check. --check exits 1 on drift.
    if args.iter().any(|a| a == "--render") {
        return match render(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--check") {
        return match check(&args) {
            Ok(drift) => {
                if drift {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--check-tools") {
        return match check_tools(&args) {
            Ok(drift) => {
                if drift {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(|a| a == "sync").unwrap_or(false) {
        return match sync_cmd(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        };
    }
    eprintln!(
        "usage: capsync sync | --render <outdir> | --check <dir> | --check-tools  [--capabilities <file>] [--catalog <dir>]"
    );
    ExitCode::FAILURE
}

/// Mirror sync.js capFileArg(): --capabilities <file>, else $HOME/personal/capabilities.md.
fn cap_file_from(args: &[String]) -> Result<PathBuf, String> {
    match flag_value(args, "--capabilities") {
        Some(v) => Ok(PathBuf::from(v)),
        None => {
            let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
            Ok(Path::new(&home).join("personal").join("capabilities.md"))
        }
    }
}

/// permissions.md sits beside the capabilities file (same personal namespace).
fn read_perms_beside(cap_file: &Path) -> String {
    let perms_file = cap_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("permissions.md");
    std::fs::read_to_string(&perms_file).unwrap_or_default()
}

/// Port of sync.js buildArtifacts(): the full ordered set of rendered artifacts as
/// (filename, content) pairs. Shared by --render (write) and --check (compare) so both see
/// identical bytes. Order mirrors the JS `files` object.
fn build_artifacts(
    args: &[String],
    cap_file: &Path,
    catalog: Option<&Path>,
    base: Option<&Path>,
) -> Vec<(String, String)> {
    let enable = parse_enable(cap_file);
    let perms_text = read_perms_beside(cap_file);

    let registry = build_registry(catalog, base);
    let (facade, direct) = split_mcp_routes(&enable, &registry);

    let bin_tools = load_bin_tools(catalog, base);
    let pipeline = pipeline_bin_arg(args);
    let bins = bin_list(&enable, &bin_tools, catalog, base, &pipeline);

    let bundle = build_skills_bundle(&enable, catalog, base);
    let authz = build_authz(&perms_text, &enable, catalog, base);

    let mut out: Vec<(String, String)> = vec![
        ("openab-agent-mcp.json".to_string(), as_cfg(&facade)),
        ("runtime-mcp.json".to_string(), as_cfg(&direct)),
        ("bin-install.tsv".to_string(), bin_install_tsv(&bins)),
        ("skills.tar.b64".to_string(), bundle.tar_b64),
        ("skills.list".to_string(), bundle.list),
    ];
    for rt in AUTHZ_RUNTIMES {
        out.push((
            format!("authz-{}.json", rt.id),
            stringify_authz(&map_authz(rt, &authz)),
        ));
    }
    out.push(("authz-suggest.txt".to_string(), suggest_text(&authz)));
    out
}

/// --render <outdir>: write every artifact off-pod (for infra to bake as a configMap).
fn render(args: &[String]) -> Result<(), String> {
    let out_dir = flag_value(args, "--render")
        .filter(|v| !v.starts_with("--"))
        .ok_or("usage: capsync --render <outdir> [--capabilities <file>] [--catalog <dir>]")?;
    let cap_file = cap_file_from(args)?;
    let catalog = resolve_catalog(args);
    let base = personal_base(&cap_file);
    let artifacts = build_artifacts(args, &cap_file, catalog.as_deref(), base.as_deref());

    std::fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {out_dir}: {e}"))?;
    let out = Path::new(&out_dir);
    for (name, content) in &artifacts {
        std::fs::write(out.join(name), content).map_err(|e| format!("write {name}: {e}"))?;
    }
    println!("rendered {} artifact(s) -> {}", artifacts.len(), out_dir);
    Ok(())
}

/// --check <dir>: re-render and compare against committed artifacts. Deterministic, no HOME
/// writes. Returns Ok(true) on drift (→ exit 1), Ok(false) if in sync. Port of sync.js check().
fn check(args: &[String]) -> Result<bool, String> {
    let dir = flag_value(args, "--check")
        .filter(|v| !v.starts_with("--"))
        .ok_or("usage: capsync --check <committed-artifacts-dir> [--capabilities <file>] [--catalog <dir>]")?;
    let cap_file = cap_file_from(args)?;
    let catalog = resolve_catalog(args);
    let base = personal_base(&cap_file);
    let artifacts = build_artifacts(args, &cap_file, catalog.as_deref(), base.as_deref());

    let dir = Path::new(&dir);
    let mut drift = false;
    for (name, want) in &artifacts {
        let committed = dir.join(name);
        match std::fs::read_to_string(&committed) {
            Ok(ref have) if have == want => println!("  {name}: IN SYNC"),
            Ok(have) => {
                drift = true;
                eprintln!("  {name}: DRIFT — committed differs from freshly rendered");
                // minimal line-level hint (mirror sync.js)
                let w: Vec<&str> = want.split('\n').collect();
                let h: Vec<&str> = have.split('\n').collect();
                for i in 0..w.len().max(h.len()) {
                    if w.get(i) != h.get(i) {
                        if let Some(hl) = h.get(i) {
                            eprintln!("      - committed: {hl}");
                        }
                        if let Some(wl) = w.get(i) {
                            eprintln!("      + rendered:  {wl}");
                        }
                    }
                }
            }
            Err(_) => {
                drift = true;
                eprintln!("  {name}: DRIFT — missing in {}", dir.display());
            }
        }
    }
    println!(
        "{}",
        if drift {
            "\ndrift detected — re-render and commit the artifacts"
        } else {
            "\nno drift — committed artifacts are current"
        }
    );
    Ok(drift)
}

// ----------------------------------------------------------------------------
// live `sync` projection (ADR 0008 Phase 1e) — mutates $HOME to project the enabled subset
// into installed runtimes. Phase 1e-1 covers the SKILLS axis only (symlink each enabled skill
// into each runtime's skills dir); MCP merge + hooks land in later 1e sub-slices, so this is
// a PARTIAL sync that runs parallel to node's sync.js (never the sole projector yet).
// ----------------------------------------------------------------------------

// (id, runtime base dir, skills dir) relative to $HOME. base must exist for the runtime to be
// considered installed; skills go under skillsDir. Mirrors sync.js SKILL_RUNTIMES.
const SKILL_RUNTIMES: &[(&str, &[&str], &[&str])] = &[
    ("claude-code", &[".claude"], &[".claude", "skills"]),
    ("codex", &[".codex"], &[".codex", "skills"]),
    (
        "antigravity",
        &[".gemini"],
        &[".gemini", "antigravity-cli", "skills"],
    ),
    (
        "opencode",
        &[".config", "opencode"],
        &[".config", "opencode", "skills"],
    ),
];

fn join_all(base: &Path, parts: &[&str]) -> PathBuf {
    let mut p = base.to_path_buf();
    for c in parts {
        p.push(c);
    }
    p
}

/// JS skillSource: catalog → <catalog>/skills/<name>; personal → <base>/skills/<name>.
fn skill_source(s: &EnableSkill, catalog: Option<&Path>, base: Option<&Path>) -> Option<PathBuf> {
    match s.source.as_str() {
        "catalog" => catalog.map(|c| c.join("skills").join(&s.name)),
        "personal" => base.map(|b| b.join("skills").join(&s.name)),
        _ => None,
    }
}

/// JS linkSkill: symlink <skills_dir>/<name> → src. up-to-date if it already points there;
/// CONFLICT (skip) if a non-symlink is in the way; otherwise (re)create the symlink.
fn link_skill(rt_base: &Path, skills_dir: &Path, name: &str, src: &Path) -> String {
    if !rt_base.exists() {
        return "skip (runtime not installed)".to_string();
    }
    let _ = std::fs::create_dir_all(skills_dir);
    let dest = skills_dir.join(name);
    if let Ok(md) = std::fs::symlink_metadata(&dest) {
        if md.file_type().is_symlink() {
            if std::fs::read_link(&dest).ok().as_deref() == Some(src) {
                return "up-to-date".to_string();
            }
            // symlink to a different target → fall through and relink
        } else {
            return format!(
                "CONFLICT: {} exists and is not our symlink — skipped",
                dest.display()
            );
        }
    }
    let _ = std::fs::remove_file(&dest); // force-remove a stale symlink/file
    match std::os::unix::fs::symlink(src, &dest) {
        Ok(()) => format!("linked → {}", src.display()),
        Err(e) => format!("ERROR symlink failed: {e}"),
    }
}

/// Upsert key→val into a JSON object's entries: overwrite in place on collision (preserving
/// position), else append. Mirrors JS `obj[key] = val`.
fn json_upsert(entries: &mut Vec<(String, Json)>, key: &str, val: Json) {
    if let Some(e) = entries.iter_mut().find(|(k, _)| k == key) {
        e.1 = val;
    } else {
        entries.push((key.to_string(), val));
    }
}

/// Port of oab-facade projectMcp: SAFE read-modify-write of ~/.openab/agent/mcp.json — parse
/// existing (lossless), .bak backup, set only our server keys under `mcpServers` via
/// shapeServer (secrets stay ${env:} refs), re-emit as JSON.stringify(_, null, 2)+"\n".
fn project_facade(servers: &[Server], home: &Path) -> Result<(), String> {
    let file = home.join(".openab").join("agent").join("mcp.json");
    let mut cfg = if file.exists() {
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let mut bak = file.clone().into_os_string();
        bak.push(".bak");
        let _ = std::fs::copy(&file, PathBuf::from(bak));
        match parse_json(&text) {
            Ok(j @ Json::Obj(_)) => j,
            _ => Json::Obj(Vec::new()),
        }
    } else {
        Json::Obj(Vec::new())
    };
    let top = match &mut cfg {
        Json::Obj(e) => e,
        _ => unreachable!(),
    };
    // cfg.mcpServers = cfg.mcpServers || {}
    if !top.iter().any(|(k, _)| k == "mcpServers") {
        top.push(("mcpServers".to_string(), Json::Obj(Vec::new())));
    }
    let ms = top.iter_mut().find(|(k, _)| k == "mcpServers").unwrap();
    if !matches!(ms.1, Json::Obj(_)) {
        ms.1 = Json::Obj(Vec::new());
    }
    if let Json::Obj(inner) = &mut ms.1 {
        for s in servers {
            json_upsert(inner, &s.name, shape_server(s));
        }
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut out = stringify(&cfg, 0);
    out.push('\n');
    std::fs::write(&file, out).map_err(|e| format!("write {}: {e}", file.display()))?;
    Ok(())
}

// ---- secret resolver (port parse.js loadDotenv/resolveRef/resolveServer) — used by the
// direct MCP route. Fixtures use only unset `env:` refs → deterministic empty values; the
// op:// / vault: / keychain: backends shell out (never exercised by the parity gate). ----

/// A resolved MCP server: registry fields with env/args/url/headers resolved for direct route.
struct ResolvedServer {
    name: String,
    transport: Option<String>,
    url: Option<String>,
    command: Option<String>,
    args: Vec<Json>,
    env: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    header_env: Vec<(String, String)>, // header key → env var NAME (codex env_http_headers)
}

/// JS runCli: capture stdout (trimmed) or None on missing binary / non-zero exit.
fn run_cli(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// JS: val.replace(/\$\{(\w+)\}/g, v => env[v] ?? process.env[v] ?? (keep_unset ? "${v}" : "")).
fn replace_braces(val: &str, env: &HashMap<String, String>, keep_unset: bool) -> String {
    let chars: Vec<char> = val.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            if let Some(j) = (i + 2..chars.len()).find(|&k| chars[k] == '}') {
                let name: String = chars[i + 2..j].iter().collect();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    match env
                        .get(&name)
                        .cloned()
                        .or_else(|| std::env::var(&name).ok())
                    {
                        Some(v) => out.push_str(&v),
                        None => {
                            if keep_unset {
                                out.push_str("${");
                                out.push_str(&name);
                                out.push('}');
                            }
                        }
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// JS resolveRef: op:///vault:/keychain: backends; env:NAME from dotenv/process.env; else a
/// `${VAR}` substitution. None = unresolved (caller keeps the ref / substitutes empty).
fn resolve_ref(val: &str, env: &HashMap<String, String>) -> Option<String> {
    if val.starts_with("op://") {
        return run_cli("op", &["read", val]);
    }
    if let Some(body) = val.strip_prefix("vault:") {
        let hash = body.rfind('#')?;
        let (secret_path, field) = (&body[..hash], &body[hash + 1..]);
        if secret_path.is_empty() || field.is_empty() {
            return None;
        }
        return run_cli(
            "vault",
            &["kv", "get", &format!("-field={field}"), secret_path],
        );
    }
    if let Some(body) = val.strip_prefix("keychain:") {
        let (service, account) = match body.find('/') {
            Some(s) => (&body[..s], Some(&body[s + 1..])),
            None => (body, None),
        };
        if service.is_empty() {
            return None;
        }
        let mut args = vec!["find-generic-password", "-s", service];
        if let Some(a) = account {
            args.push("-a");
            args.push(a);
        }
        args.push("-w");
        return run_cli("security", &args);
    }
    if let Some(name) = val.strip_prefix("env:").filter(|n| !n.is_empty()) {
        return env.get(name).cloned().or_else(|| std::env::var(name).ok());
    }
    Some(replace_braces(val, env, false)) // else: ${VAR} → value or "" (unset)
}

/// JS loadDotenv: parse <catalog>/secrets/.env KEY=VALUE lines (skip comments). Empty if absent.
fn load_dotenv(catalog: Option<&Path>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(c) = catalog else { return out };
    let Ok(text) = std::fs::read_to_string(c.join("secrets").join(".env")) else {
        return out;
    };
    for l in text.split('\n') {
        if l.trim().starts_with('#') {
            continue;
        }
        if let Some(eq) = l.find('=') {
            let key = &l[..eq];
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(key.to_string(), l[eq + 1..].to_string());
            }
        }
    }
    out
}

/// JS resolveServer: resolve env (unresolved → ""), args (unresolved → keep original), url
/// (unset ${VAR} kept), and headers (env: refs resolved-or-"", literals passed through).
fn resolve_server(s: &Server, env: &HashMap<String, String>) -> ResolvedServer {
    let resolved_env = s
        .env
        .iter()
        .map(|(k, v)| (k.clone(), resolve_ref(v, env).unwrap_or_default()))
        .collect();
    let args = match s.args.clone().unwrap_or(Json::Arr(Vec::new())) {
        Json::Arr(items) => items
            .into_iter()
            .map(|it| match &it {
                Json::Str(a) => match resolve_ref(a, env) {
                    Some(rv) => Json::Str(rv),
                    None => it.clone(),
                },
                _ => it,
            })
            .collect(),
        _ => Vec::new(),
    };
    let url = s.url.as_ref().map(|u| replace_braces(u, env, true));
    let mut headers = Vec::new();
    let mut header_env = Vec::new();
    for (k, v) in &s.headers {
        if let Some(name) = v.strip_prefix("env:").filter(|n| !n.is_empty()) {
            header_env.push((k.clone(), name.to_string()));
            headers.push((k.clone(), resolve_ref(v, env).unwrap_or_default()));
        } else {
            headers.push((k.clone(), v.clone()));
        }
    }
    ResolvedServer {
        name: s.name.clone(),
        transport: s.transport.clone(),
        url,
        command: s.command.clone(),
        args,
        env: resolved_env,
        headers,
        header_env,
    }
}

// ---- direct MCP projectors (Phase 1e-2b): claude-code / antigravity / opencode JSON RMW.
// codex (TOML managed-block) lands in 1e-2c. Each SAFE-merges only our server keys. ----

fn obj_of(pairs: &[(String, String)]) -> Json {
    Json::Obj(
        pairs
            .iter()
            .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
            .collect(),
    )
}

/// Read a JSON config for read-modify-write: parse existing (lossless) + .bak, or a fresh {}.
/// Returns (cfg, existed).
fn json_read_for_rmw(file: &Path) -> Result<(Json, bool), String> {
    if file.exists() {
        let text =
            std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let mut bak = file.to_path_buf().into_os_string();
        bak.push(".bak");
        let _ = std::fs::copy(file, PathBuf::from(bak));
        let cfg = match parse_json(&text) {
            Ok(j @ Json::Obj(_)) => j,
            _ => Json::Obj(Vec::new()),
        };
        Ok((cfg, true))
    } else {
        Ok((Json::Obj(Vec::new()), false))
    }
}

fn json_write_pretty(file: &Path, cfg: &Json) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut out = stringify(cfg, 0);
    out.push('\n');
    std::fs::write(file, out).map_err(|e| format!("write {}: {e}", file.display()))
}

/// Set cfg[container][name] = entry(server) for each server (SAFE upsert, container created if
/// missing / coerced to an object).
fn upsert_servers(
    cfg: &mut Json,
    container: &str,
    servers: &[ResolvedServer],
    entry: impl Fn(&ResolvedServer) -> Json,
) {
    let top = match cfg {
        Json::Obj(e) => e,
        _ => return,
    };
    if !top.iter().any(|(k, _)| k == container) {
        top.push((container.to_string(), Json::Obj(Vec::new())));
    }
    let c = top.iter_mut().find(|(k, _)| k == container).unwrap();
    if !matches!(c.1, Json::Obj(_)) {
        c.1 = Json::Obj(Vec::new());
    }
    if let Json::Obj(inner) = &mut c.1 {
        for s in servers {
            json_upsert(inner, &s.name, entry(s));
        }
    }
}

// stdio entry shared by claude-code + antigravity: {command?, args, env}.
fn stdio_json_entry(s: &ResolvedServer) -> Vec<(String, Json)> {
    let mut e = Vec::new();
    if let Some(c) = &s.command {
        e.push(("command".to_string(), Json::Str(c.clone())));
    }
    e.push(("args".to_string(), Json::Arr(s.args.clone())));
    e.push(("env".to_string(), obj_of(&s.env)));
    e
}

fn claude_entry(s: &ResolvedServer) -> Json {
    if s.transport.as_deref() == Some("http") {
        let mut e = vec![("type".to_string(), Json::Str("http".to_string()))];
        if let Some(u) = &s.url {
            e.push(("url".to_string(), Json::Str(u.clone())));
        }
        if !s.headers.is_empty() {
            e.push(("headers".to_string(), obj_of(&s.headers)));
        }
        Json::Obj(e)
    } else {
        Json::Obj(stdio_json_entry(s))
    }
}

fn antigravity_entry(s: &ResolvedServer) -> Json {
    if s.transport.as_deref() == Some("http") {
        let mut e = Vec::new();
        if let Some(u) = &s.url {
            e.push(("serverUrl".to_string(), Json::Str(u.clone())));
        }
        if !s.headers.is_empty() {
            e.push(("headers".to_string(), obj_of(&s.headers)));
        }
        Json::Obj(e)
    } else {
        Json::Obj(stdio_json_entry(s))
    }
}

fn opencode_entry(s: &ResolvedServer) -> Json {
    let t = s.transport.as_deref();
    if t == Some("http") || t == Some("sse") {
        let mut e = vec![("type".to_string(), Json::Str("remote".to_string()))];
        if let Some(u) = &s.url {
            e.push(("url".to_string(), Json::Str(u.clone())));
        }
        e.push(("enabled".to_string(), Json::Bool(true)));
        if !s.headers.is_empty() {
            e.push(("headers".to_string(), obj_of(&s.headers)));
        }
        Json::Obj(e)
    } else {
        let mut cmd = vec![Json::Str(s.command.clone().unwrap_or_default())];
        cmd.extend(s.args.clone());
        let mut e = vec![
            ("type".to_string(), Json::Str("local".to_string())),
            ("command".to_string(), Json::Arr(cmd)),
            ("enabled".to_string(), Json::Bool(true)),
        ];
        if !s.env.is_empty() {
            e.push(("environment".to_string(), obj_of(&s.env)));
        }
        Json::Obj(e)
    }
}

fn project_claude(direct: &[ResolvedServer], home: &Path) -> Result<(), String> {
    let file = home.join(".claude.json");
    if !(home.join(".claude").exists() || file.exists()) {
        return Ok(()); // not installed
    }
    let (mut cfg, _) = json_read_for_rmw(&file)?;
    upsert_servers(&mut cfg, "mcpServers", direct, claude_entry);
    json_write_pretty(&file, &cfg)
}

fn project_antigravity(direct: &[ResolvedServer], home: &Path) -> Result<(), String> {
    if !home.join(".gemini").exists() {
        return Ok(());
    }
    let file = home.join(".gemini").join("config").join("mcp_config.json");
    let (mut cfg, _) = json_read_for_rmw(&file)?;
    upsert_servers(&mut cfg, "mcpServers", direct, antigravity_entry);
    json_write_pretty(&file, &cfg)
}

fn project_opencode(direct: &[ResolvedServer], home: &Path) -> Result<(), String> {
    let dir = home.join(".config").join("opencode");
    let file = dir.join("opencode.json");
    if !(dir.exists() || file.exists()) {
        return Ok(());
    }
    let (mut cfg, existed) = json_read_for_rmw(&file)?;
    if !existed {
        if let Json::Obj(e) = &mut cfg {
            e.push((
                "$schema".to_string(),
                Json::Str("https://opencode.ai/config.json".to_string()),
            ));
        }
    }
    upsert_servers(&mut cfg, "mcp", direct, opencode_entry);
    json_write_pretty(&file, &cfg)
}

// ---- codex direct projector (Phase 1e-2c) — ~/.codex/config.toml managed block ----
const CODEX_BEGIN: &str = "# >>> agents-shared-capabilities (managed) — do not edit by hand";
const CODEX_END: &str = "# <<< agents-shared-capabilities";

/// JS q(): '"' + v.replace(/\\/g,'\\\\').replace(/"/g,'\\"') + '"'.
fn toml_q(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// JS String(v) for an arg value, as codex's q() sees it (numbers/bools become strings).
fn json_scalar_string(j: &Json) -> String {
    match j {
        Json::Str(s) => s.clone(),
        Json::Num(n) => n.clone(),
        Json::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
        Json::Null => "null".to_string(),
        _ => String::new(),
    }
}

/// Port of codex.js toToml: one `[mcp_servers.<name>]` table per server between markers.
fn codex_to_toml(servers: &[ResolvedServer]) -> String {
    let mut out = vec![CODEX_BEGIN.to_string()];
    for s in servers {
        out.push(format!("[mcp_servers.{}]", s.name));
        if s.transport.as_deref() == Some("http") {
            out.push(format!(
                "url = {}",
                toml_q(s.url.as_deref().unwrap_or("undefined"))
            ));
            let env_keys: HashSet<&str> = s.header_env.iter().map(|(k, _)| k.as_str()).collect();
            let lit: Vec<String> = s
                .headers
                .iter()
                .filter(|(k, _)| !env_keys.contains(k.as_str()))
                .map(|(k, v)| format!("{} = {}", toml_q(k), toml_q(v)))
                .collect();
            if !lit.is_empty() {
                out.push(format!("http_headers = {{ {} }}", lit.join(", ")));
            }
            let env_h: Vec<String> = s
                .header_env
                .iter()
                .map(|(k, v)| format!("{} = {}", toml_q(k), toml_q(v)))
                .collect();
            if !env_h.is_empty() {
                out.push(format!("env_http_headers = {{ {} }}", env_h.join(", ")));
            }
        } else {
            out.push(format!(
                "command = {}",
                toml_q(s.command.as_deref().unwrap_or("undefined"))
            ));
            let args: Vec<String> = s
                .args
                .iter()
                .map(|a| toml_q(&json_scalar_string(a)))
                .collect();
            out.push(format!("args = [{}]", args.join(", ")));
            let env: Vec<&(String, String)> = s.env.iter().filter(|(_, v)| !v.is_empty()).collect();
            if !env.is_empty() {
                out.push(format!("[mcp_servers.{}.env]", s.name));
                for (k, v) in env {
                    out.push(format!("{} = {}", k, toml_q(v)));
                }
            }
        }
        out.push(String::new()); // blank line after each server
    }
    out.push(CODEX_END.to_string());
    out.join("\n")
}

/// JS: text.replace(/\n*BEGIN[\s\S]*?END/g, '') — remove each managed block (+ leading newlines).
fn strip_managed_block(text: &str, begin: &str, end: &str) -> String {
    let mut s = text.to_string();
    while let Some(bpos) = s.find(begin) {
        let Some(erel) = s[bpos..].find(end) else {
            break;
        };
        let eend = bpos + erel + end.len();
        let mut start = bpos;
        while start > 0 && s.as_bytes()[start - 1] == b'\n' {
            start -= 1;
        }
        s.replace_range(start..eend, "");
    }
    s
}

/// Port of codex.js projectMcp: strip the old managed block, append a freshly generated one.
fn project_codex(direct: &[ResolvedServer], home: &Path) -> Result<(), String> {
    let codex = home.join(".codex");
    if !codex.exists() {
        return Ok(()); // not installed
    }
    let file = codex.join("config.toml");
    let text = if file.exists() {
        let t =
            std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let mut bak = file.clone().into_os_string();
        bak.push(".bak");
        let _ = std::fs::copy(&file, PathBuf::from(bak));
        t
    } else {
        String::new()
    };
    let stripped = strip_managed_block(&text, CODEX_BEGIN, CODEX_END);
    let stripped = stripped.trim_end(); // JS .replace(/\s+$/, '')
    let block = codex_to_toml(direct);
    let next = if stripped.is_empty() {
        format!("{block}\n")
    } else {
        format!("{stripped}\n\n{block}\n")
    };
    std::fs::create_dir_all(&codex).map_err(|e| format!("mkdir {}: {e}", codex.display()))?;
    std::fs::write(&file, next).map_err(|e| format!("write {}: {e}", file.display()))
}

// ---- hooks projection (Phase 1e-3) — port of parse.js parseHookRegistry + sync.js
// buildHookRegistry + claude-code/antigravity projectHooks. Only effect=allow is projected;
// a now-disabled hook is stripped from configs that still carry a previously-managed entry. ----

#[derive(Clone, Default)]
struct HookDef {
    name: String,
    event: Option<String>,
    matcher: Option<String>,
    command: Option<String>,
}

fn parse_hook_registry(text: &str) -> Vec<HookDef> {
    let mut hooks: Vec<HookDef> = Vec::new();
    let mut section: Option<String> = None; // "capability" | ...
    for raw in text.split('\n') {
        let line = trim_end(raw);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= 2 {
            if let Some(name) = match_item_name(t) {
                hooks.push(HookDef {
                    name: strip(&name),
                    ..Default::default()
                });
                section = None;
                continue;
            }
        }
        let cur = match hooks.last_mut() {
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
        if section.is_some() && indent >= 6 {
            continue; // nested (capability) — not needed for projection
        }
        section = None;
        match key.as_str() {
            "event" => cur.event = Some(strip(&val)),
            "matcher" => cur.matcher = Some(strip(&val)),
            "command" => cur.command = Some(strip(&val)),
            _ => {}
        }
    }
    hooks
}

fn load_hook_registry(dir: &Path) -> Vec<HookDef> {
    match std::fs::read_to_string(dir.join("hooks").join("registry.yaml")) {
        Ok(t) => parse_hook_registry(&t),
        Err(_) => Vec::new(),
    }
}

fn build_hook_registry(catalog: Option<&Path>, base: Option<&Path>) -> HashMap<String, HookDef> {
    let mut map = HashMap::new();
    if let Some(c) = catalog {
        for h in load_hook_registry(c) {
            map.insert(h.name.clone(), h);
        }
    }
    if let Some(b) = base {
        for h in load_hook_registry(b) {
            map.insert(h.name.clone(), h);
        }
    }
    map
}

// canonical event → runtime-native event. None = unmapped (skip), mirroring HOOK_EVENT.
fn claude_hook_event(ev: &str) -> Option<&'static str> {
    match ev {
        "pre-tool" => Some("PreToolUse"),
        "post-tool" => Some("PostToolUse"),
        "session-start" => Some("SessionStart"),
        "stop" => Some("Stop"),
        "user-prompt-submit" => Some("UserPromptSubmit"),
        _ => None,
    }
}
fn antigravity_hook_event(ev: &str) -> Option<&'static str> {
    match ev {
        "pre-tool" => Some("PreToolUse"),
        "post-tool" => Some("PostToolUse"),
        "stop" => Some("Stop"),
        _ => None,
    }
}

fn json_get<'a>(obj: &'a Json, key: &str) -> Option<&'a Json> {
    match obj {
        Json::Obj(e) => e.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}
fn json_get_mut<'a>(obj: &'a mut Json, key: &str) -> Option<&'a mut Json> {
    match obj {
        Json::Obj(e) => e.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

// one hook entry's group: { [matcher?], hooks: [entry] }.
fn hook_group(matcher: &Option<String>, entry: Json) -> Json {
    let mut g = Vec::new();
    if let Some(m) = matcher {
        g.push(("matcher".to_string(), Json::Str(m.clone())));
    }
    g.push(("hooks".to_string(), Json::Arr(vec![entry])));
    Json::Obj(g)
}

const HOOK_MANAGED: &str = "agents-shared-capabilities";

/// Port of claude-code projectHooks: ~/.claude/settings.json `hooks`. Strip our previously
/// managed entries (`_managedBy`), then add the enabled set. Idempotent.
fn project_claude_hooks(hooks: &[HookDef], home: &Path) -> Result<(), String> {
    let file = home.join(".claude").join("settings.json");
    let exists = file.exists();
    if !exists && hooks.is_empty() {
        return Ok(()); // don't create an empty settings.json
    }
    let (mut cfg, _) = if exists {
        json_read_for_rmw(&file)?
    } else {
        (Json::Obj(Vec::new()), false)
    };
    if let Json::Obj(top) = &mut cfg {
        if !top.iter().any(|(k, _)| k == "hooks") {
            top.push(("hooks".to_string(), Json::Obj(Vec::new())));
        }
    }
    // 1. strip previously-managed entries, dropping now-empty groups/events.
    if let Some(Json::Obj(hmap)) = json_get_mut(&mut cfg, "hooks") {
        for (_ev, groups) in hmap.iter_mut() {
            if let Json::Arr(gs) = groups {
                let managed = Json::Str(HOOK_MANAGED.to_string());
                let kept: Vec<Json> = gs
                    .drain(..)
                    .filter_map(|g| {
                        let Json::Obj(gentries) = &g else { return None };
                        let filtered: Vec<Json> = match gentries.iter().find(|(k, _)| k == "hooks")
                        {
                            Some((_, Json::Arr(items))) => items
                                .iter()
                                .filter(|h| json_get(h, "_managedBy") != Some(&managed))
                                .cloned()
                                .collect(),
                            _ => Vec::new(),
                        };
                        if filtered.is_empty() {
                            return None;
                        }
                        let mut ng = gentries.clone();
                        json_upsert(&mut ng, "hooks", Json::Arr(filtered));
                        Some(Json::Obj(ng))
                    })
                    .collect();
                *groups = Json::Arr(kept);
            }
        }
        hmap.retain(|(_k, v)| !matches!(v, Json::Arr(a) if a.is_empty()));
    }
    // 2. add enabled hooks.
    for h in hooks {
        let Some(native) = h.event.as_deref().and_then(claude_hook_event) else {
            continue;
        };
        let mut entry = vec![("type".to_string(), Json::Str("command".to_string()))];
        if let Some(c) = &h.command {
            entry.push(("command".to_string(), Json::Str(c.clone())));
        }
        entry.push((
            "_managedBy".to_string(),
            Json::Str(HOOK_MANAGED.to_string()),
        ));
        entry.push(("_hook".to_string(), Json::Str(h.name.clone())));
        let group = hook_group(&h.matcher, Json::Obj(entry));
        if let Some(Json::Obj(hmap)) = json_get_mut(&mut cfg, "hooks") {
            if !hmap.iter().any(|(k, _)| k == native) {
                hmap.push((native.to_string(), Json::Arr(Vec::new())));
            }
            if let Some((_, Json::Arr(arr))) = hmap.iter_mut().find(|(k, _)| k == native) {
                arr.push(group);
            }
        }
    }
    // if the hooks map ended up empty, drop the key entirely.
    if let Json::Obj(top) = &mut cfg {
        let empty = matches!(top.iter().find(|(k, _)| k == "hooks"), Some((_, Json::Obj(e))) if e.is_empty());
        if empty {
            top.retain(|(k, _)| k != "hooks");
        }
    }
    json_write_pretty(&file, &cfg)
}

/// Port of antigravity projectHooks: ~/.gemini/config/hooks.json keyed by `asc:<name>`.
/// Strip our `asc:` keys, then re-add the enabled set.
fn project_antigravity_hooks(hooks: &[HookDef], home: &Path) -> Result<(), String> {
    let file = home.join(".gemini").join("config").join("hooks.json");
    let exists = file.exists();
    if !exists && hooks.is_empty() {
        return Ok(());
    }
    let (mut cfg, _) = if exists {
        json_read_for_rmw(&file)?
    } else {
        (Json::Obj(Vec::new()), false)
    };
    if let Json::Obj(top) = &mut cfg {
        top.retain(|(k, _)| !k.starts_with("asc:"));
    }
    for h in hooks {
        let Some(native) = h.event.as_deref().and_then(antigravity_hook_event) else {
            continue;
        };
        let mut entry = vec![("type".to_string(), Json::Str("command".to_string()))];
        if let Some(c) = &h.command {
            entry.push(("command".to_string(), Json::Str(c.clone())));
        }
        let group = hook_group(&h.matcher, Json::Obj(entry));
        let value = Json::Obj(vec![(native.to_string(), Json::Arr(vec![group]))]);
        if let Json::Obj(top) = &mut cfg {
            json_upsert(top, &format!("asc:{}", h.name), value);
        }
    }
    json_write_pretty(&file, &cfg)
}

/// `capsync sync`: live projection into $HOME. Phase 1e covers skills symlink (1e-1), MCP
/// facade + direct all four runtimes (1e-2), and hooks (1e-3) — full parity with sync.js's
/// live path. Reads $HOME/personal/capabilities.md (like sync.js, NOT --capabilities).
fn sync_cmd(args: &[String]) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "HOME not set".to_string())?);
    let catalog = resolve_catalog(args);
    let cap_file = home.join("personal").join("capabilities.md");
    let enable = parse_enable(&cap_file);
    // JS personalBase(): dirname(realpath($HOME/personal)) == personal_base of its capabilities.md.
    let base = personal_base(&cap_file);

    let listed = if enable.skills.is_empty() {
        "(none)".to_string()
    } else {
        enable
            .skills
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("enabled skills: {listed}");
    for (id, base_c, skills_c) in SKILL_RUNTIMES {
        let rt_base = join_all(&home, base_c);
        let skills_dir = join_all(&home, skills_c);
        for s in &enable.skills {
            let src = match skill_source(s, catalog.as_deref(), base.as_deref()) {
                Some(p) => p,
                None => {
                    println!(
                        "  [{id}] {}: ERROR cannot resolve source={}",
                        s.name, s.source
                    );
                    continue;
                }
            };
            if !src.exists() {
                println!(
                    "  [{id}] {}: ERROR source missing ({})",
                    s.name,
                    src.display()
                );
                continue;
            }
            println!(
                "  [{id}] {}: {}",
                s.name,
                link_skill(&rt_base, &skills_dir, &s.name, &src)
            );
        }
    }

    // ---- MCP: split by route. direct → resolve secrets + project into runtime configs
    // (claude/antigravity/opencode in 1e-2b; codex TOML in 1e-2c). facade → oab-facade (1e-2a).
    // Mirror sync.js: when any MCP is enabled, the direct projectors run (even with an empty
    // direct list), each self-gating on whether its runtime is installed. ----
    if !enable.mcp.is_empty() {
        let registry = build_registry(catalog.as_deref(), base.as_deref());
        let env = load_dotenv(catalog.as_deref());
        let mut direct: Vec<ResolvedServer> = Vec::new();
        let mut facade: Vec<Server> = Vec::new();
        for want in &enable.mcp {
            match registry.get(&want.name) {
                None => println!("  mcp {}: ERROR not in registry", want.name),
                Some(def) => {
                    if def.route.as_deref().unwrap_or(DEFAULT_ROUTE) == "direct" {
                        direct.push(resolve_server(def, &env));
                        println!("  mcp {}: route=direct", want.name);
                    } else {
                        facade.push(def.clone());
                        println!("  mcp {}: route=facade", want.name);
                    }
                }
            }
        }
        // direct projectors (claude / codex / antigravity / opencode — mirrors sync.js order)
        project_claude(&direct, &home)?;
        project_codex(&direct, &home)?;
        project_antigravity(&direct, &home)?;
        project_opencode(&direct, &home)?;
        if !facade.is_empty() {
            project_facade(&facade, &home)?;
        }
        println!("[mcp] direct: {} · facade: {}", direct.len(), facade.len());
    }

    // ---- hooks (ADR 0005) — only effect=allow is projected; the projectors always run (even
    // with an empty set) so a now-disabled hook is stripped. Each gates on runtime install. ----
    let resolved_hooks: Vec<HookDef> = {
        let registry = build_hook_registry(catalog.as_deref(), base.as_deref());
        enable
            .hooks
            .iter()
            .filter(|h| h.effect == "allow")
            .filter_map(|want| match registry.get(&want.name) {
                Some(def) => Some(def.clone()),
                None => {
                    println!("  hook {}: ERROR not in hook registry", want.name);
                    None
                }
            })
            .collect()
    };
    if home.join(".claude").exists() || home.join(".claude.json").exists() {
        project_claude_hooks(&resolved_hooks, &home)?;
    }
    if home.join(".gemini").exists() {
        project_antigravity_hooks(&resolved_hooks, &home)?;
    }
    println!("[hooks] projected: {}", resolved_hooks.len());
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

struct Suggestion {
    tool: String,
    command: String,
}

struct Authz {
    flag: String,
    allow: Vec<String>, // sorted, deny-subtracted
    deny: Vec<String>,  // sorted
    suggestions: Vec<Suggestion>,
}

/// Port of buildAuthz (ADR 0007). explicit mode: allow = permissions.md `allow` minus deny.
/// auto mode (`authorize_skill_requires: auto`): additionally allow the commands the enabled
/// skills' `requires` resolve to. deny always wins. `suggestions` is the requires-derived hint
/// list (used by suggest_text → authz-suggest.txt), sorted by command.
fn build_authz(
    perms_text: &str,
    enable: &Enable,
    catalog: Option<&Path>,
    base: Option<&Path>,
) -> Authz {
    let perms = parse_permissions(perms_text);
    let suggestions = required_commands(enable, catalog, base);
    let deny: BTreeSet<String> = perms.deny.iter().cloned().collect();
    let mut allow_set: BTreeSet<String> = perms.allow.iter().cloned().collect();
    if perms.flag == "auto" {
        for s in &suggestions {
            allow_set.insert(s.command.clone());
        }
    }
    let allow: Vec<String> = js_sort(
        allow_set
            .into_iter()
            .filter(|c| !deny.contains(c))
            .collect(),
    );
    Authz {
        flag: perms.flag,
        allow,
        deny: js_sort(deny.into_iter().collect()),
        suggestions,
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

#[derive(Clone, PartialEq)]
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

/// Enabled subset from capabilities.md. Phase 1a used only `mcp`; Phase 1b adds `skills`
/// (name + source) for the bin `requires` closure. hooks land with their own slice.
/// Mirrors parse.js parseEnable's `skills:` and `mcp.servers` handling.
#[derive(Default)]
struct Enable {
    skills: Vec<EnableSkill>,
    mcp: Vec<EnableMcp>,
    hooks: Vec<EnableHook>,
}
struct EnableSkill {
    name: String,
    source: String, // "catalog" | "personal"
}
struct EnableMcp {
    name: String,
}
struct EnableHook {
    name: String,
    effect: String, // "allow" | "deny" (default allow)
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
        match top.as_deref() {
            Some("skills") => {
                if let Some(name) = match_skill_name(raw) {
                    out.skills.push(EnableSkill {
                        name,
                        source: "catalog".to_string(),
                    });
                } else if let Some(src) = match_source(raw) {
                    if let Some(last) = out.skills.last_mut() {
                        last.source = src;
                    }
                }
            }
            Some("mcp") => {
                if is_servers_line(raw) {
                    mcp_servers = true;
                } else if mcp_servers {
                    if let Some(name) = match_dash_name(raw) {
                        out.mcp.push(EnableMcp { name });
                    }
                }
            }
            Some("hooks") => {
                if let Some((name, effect)) = match_hook_inline(raw) {
                    out.hooks.push(EnableHook { name, effect });
                } else if let Some(effect) = match_effect(raw) {
                    if let Some(last) = out.hooks.last_mut() {
                        last.effect = effect;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// JS: /^\s*-\s*\{?\s*name:\s*([A-Za-z0-9_-]+)(?:.*effect:\s*(allow|deny))?/ — hook item
/// (block or inline), effect defaults to "allow".
fn match_hook_inline(raw: &str) -> Option<(String, String)> {
    let r = raw.trim_start();
    let r = r.strip_prefix('-')?;
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix('{').unwrap_or(r);
    let r = r.trim_start_matches([' ', '\t']);
    let after = r.strip_prefix("name:")?;
    let after = after.trim_start_matches([' ', '\t']);
    let name: String = after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if name.is_empty() {
        return None;
    }
    let effect = find_effect(&after[name.len()..]).unwrap_or_else(|| "allow".to_string());
    Some((name, effect))
}

/// JS greedy `.*effect:\s*(allow|deny)` — the last `effect:` followed by allow/deny.
fn find_effect(s: &str) -> Option<String> {
    let mut result = None;
    let mut idx = 0;
    while let Some(p) = s[idx..].find("effect:") {
        let start = idx + p + "effect:".len();
        let v = s[start..].trim_start_matches([' ', '\t']);
        for kw in ["allow", "deny"] {
            if v.strip_prefix(kw).is_some() {
                result = Some(kw.to_string());
            }
        }
        idx = start;
    }
    result
}

/// JS: /^\s*effect:\s*(allow|deny)\b/ group 1 (block-form effect line).
fn match_effect(raw: &str) -> Option<String> {
    let r = raw.trim_start().strip_prefix("effect:")?;
    let r = r.trim_start_matches([' ', '\t']);
    for kw in ["allow", "deny"] {
        if let Some(after) = r.strip_prefix(kw) {
            if after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            {
                return Some(kw.to_string());
            }
        }
    }
    None
}

/// JS: raw.match(/^\s*-\s*name:\s*(\S[^\n]*?)\s*$/) group 1 — skill item name (block form;
/// value is the trimmed remainder, which may contain spaces, unlike the MCP `name` token).
fn match_skill_name(raw: &str) -> Option<String> {
    let r = raw.trim_start();
    let r = r.strip_prefix('-')?;
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix("name:")?;
    let r = r.trim_start_matches([' ', '\t']);
    let v = r.trim_end();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// JS: raw.match(/^\s*source:\s*(\S+)/) group 1.
fn match_source(raw: &str) -> Option<String> {
    let r = raw.trim_start();
    let r = r.strip_prefix("source:")?;
    let r = r.trim_start_matches([' ', '\t']);
    let v: String = r.chars().take_while(|c| !c.is_whitespace()).collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
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

// ============================================================================
// bin axis + requires closure (ADR 0008 Phase 1b) — port of parse.js parseBinRegistry /
// parseRequires / frontmatterBlock, install-bin.js loadBinTools / requiredToolNames, and
// sync.js binInstallTsv / pipelineBinArg. Drives both bin-install.tsv and (via
// required_commands) authz auto mode + authz-suggest.txt.
// ============================================================================

#[derive(Clone, Default)]
struct Platform {
    asset: Option<String>,
    sha256: Option<String>,
}

#[derive(Clone, Default)]
struct BinTool {
    name: String,
    bin: Option<String>,
    archive: Option<String>,
    url: Option<String>,
    pinned_version: Option<String>,
    platforms: Vec<(String, Platform)>, // insertion order; TSV re-sorts keys
}

/// Port of parse.js parseBinRegistry: `- name:` items (indent <= 2), 4-space scalar props,
/// a nested `platforms:` map (keys at indent 6, `asset`/`sha256` at indent >= 8).
fn parse_bin_registry(text: &str) -> Vec<BinTool> {
    let mut tools: Vec<BinTool> = Vec::new();
    let mut in_platforms = false;
    let mut plat: Option<String> = None;
    for raw in text.split('\n') {
        let line = trim_end(raw);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= 2 {
            if let Some(name) = match_item_name(t) {
                tools.push(BinTool {
                    name: strip(&name),
                    ..Default::default()
                });
                in_platforms = false;
                plat = None;
                continue;
            }
        }
        let cur = match tools.last_mut() {
            Some(c) => c,
            None => continue,
        };
        if indent == 4 && t == "platforms:" {
            in_platforms = true;
            plat = None;
            continue;
        }
        if in_platforms && indent == 6 {
            if let Some(k) = match_opener(t) {
                cur.platforms.push((k.clone(), Platform::default()));
                plat = Some(k);
                continue;
            }
        }
        if in_platforms && indent >= 8 {
            if let Some(p) = &plat {
                if let Some((key, val)) = match_kv(t) {
                    if let Some(entry) = cur.platforms.iter_mut().find(|(pk, _)| pk == p) {
                        match key.as_str() {
                            "asset" => entry.1.asset = Some(strip(&val)),
                            "sha256" => entry.1.sha256 = Some(strip(&val)),
                            _ => {}
                        }
                    }
                }
            }
            continue;
        }
        if indent <= 4 {
            in_platforms = false;
            plat = None;
            if let Some((key, val)) = match_kv(t) {
                match key.as_str() {
                    "bin" => cur.bin = Some(strip(&val)),
                    "archive" => cur.archive = Some(strip(&val)),
                    "url" => cur.url = Some(strip(&val)),
                    "pinned-version" => cur.pinned_version = Some(strip(&val)),
                    _ => {} // name (dupe), source, provenance, … don't affect the TSV
                }
            }
        }
    }
    tools
}

fn load_bin_registry(dir: &Path) -> Vec<BinTool> {
    let f = dir.join("bin").join("registry.yaml");
    match std::fs::read_to_string(&f) {
        Ok(text) => parse_bin_registry(&text),
        Err(_) => Vec::new(),
    }
}

/// JS loadBinTools: catalog then personal (personal overrides on name collision).
fn load_bin_tools(catalog: Option<&Path>, base: Option<&Path>) -> HashMap<String, BinTool> {
    let mut map: HashMap<String, BinTool> = HashMap::new();
    if let Some(c) = catalog {
        for t in load_bin_registry(c) {
            map.insert(t.name.clone(), t);
        }
    }
    if let Some(b) = base {
        for t in load_bin_registry(b) {
            map.insert(t.name.clone(), t);
        }
    }
    map
}

/// JS: text.match(/^---\n([\s\S]*?)\n---/) group 1 — the leading frontmatter block ('' if none).
fn frontmatter_block(text: &str) -> String {
    if let Some(after) = text.strip_prefix("---\n") {
        if let Some(pos) = after.find("\n---") {
            return after[..pos].to_string();
        }
    }
    String::new()
}

/// Port of parse.js parseRequires, names only (min is unused by render). Block form
/// (`requires:` then `- name:` items) or inline (`requires: [ {name: x}, … ]`).
fn parse_requires_names(fm: &str) -> Vec<String> {
    let lines: Vec<&str> = fm.split('\n').collect();
    let idx = lines.iter().position(|l| is_requires_line(l));
    match idx {
        None => {
            for l in &lines {
                if let Some(inner) = match_inline_requires(l) {
                    return extract_inline_names(&inner);
                }
            }
            Vec::new()
        }
        Some(i) => {
            let mut out = Vec::new();
            for l in &lines[i + 1..] {
                if let Some(name) = match_block_require_name(l) {
                    out.push(name);
                    continue;
                }
                if l.trim_start().starts_with("min:") {
                    continue; // min line — indented, ignored for names
                }
                if l.chars().next().is_some_and(|c| !c.is_whitespace()) {
                    break; // dedent to the next top-level key
                }
            }
            out
        }
    }
}

/// JS: /^requires:\s*(#.*)?$/ — a bare `requires:` line (optional trailing comment).
fn is_requires_line(l: &str) -> bool {
    match l.strip_prefix("requires:") {
        Some(rest) => {
            let rest = rest.trim_start_matches([' ', '\t']);
            rest.is_empty() || rest.starts_with('#')
        }
        None => false,
    }
}

/// JS: /^requires:\s*\[(.+)\]\s*$/ — the inner text of an inline `requires: [ … ]`.
fn match_inline_requires(l: &str) -> Option<String> {
    let r = l.strip_prefix("requires:")?;
    let r = r.trim_start_matches([' ', '\t']);
    let r = r.strip_prefix('[')?;
    let end = r.rfind(']')?;
    let after = &r[end + 1..];
    if after.trim_start_matches([' ', '\t']).is_empty() {
        Some(r[..end].to_string())
    } else {
        None
    }
}

/// JS global /name:\s*([A-Za-z0-9_-]+)/g over an inline requires body.
fn extract_inline_names(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = inner;
    while let Some(p) = rest.find("name:") {
        let after = rest[p + "name:".len()..].trim_start_matches([' ', '\t']);
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
        rest = &rest[p + "name:".len()..];
    }
    out
}

/// JS: /^\s*-\s*name:\s*([A-Za-z0-9_-]+)/ group 1 (a block requires item).
fn match_block_require_name(l: &str) -> Option<String> {
    let r = l.trim_start();
    let r = r.strip_prefix('-')?;
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

/// JS requiredToolNames: enabled skills' SKILL.md `requires` names, de-duped, first-seen order.
fn required_tool_names(
    enable: &Enable,
    catalog: Option<&Path>,
    base: Option<&Path>,
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for s in &enable.skills {
        let dir = match (s.source.as_str(), base) {
            ("personal", Some(b)) => b.join("skills").join(&s.name),
            _ => match catalog {
                Some(c) => c.join("skills").join(&s.name),
                None => continue,
            },
        };
        let text = match std::fs::read_to_string(dir.join("SKILL.md")) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for name in parse_requires_names(&frontmatter_block(&text)) {
            if seen.insert(name.clone()) {
                names.push(name);
            }
        }
    }
    names
}

/// JS requiredCommands: each required tool name → {tool, command=bin||name}, sorted by command.
/// localeCompare is approximated by scalar order (equal for lowercase-ascii command names;
/// the parity gate catches any divergence).
fn required_commands(
    enable: &Enable,
    catalog: Option<&Path>,
    base: Option<&Path>,
) -> Vec<Suggestion> {
    let tools = load_bin_tools(catalog, base);
    let mut sug: Vec<Suggestion> = required_tool_names(enable, catalog, base)
        .into_iter()
        .map(|name| {
            let command = tools
                .get(&name)
                .and_then(|t| t.bin.clone())
                .unwrap_or_else(|| name.clone());
            Suggestion {
                tool: name,
                command,
            }
        })
        .collect();
    sug.sort_by(|a, b| a.command.cmp(&b.command));
    sug
}

/// JS suggestText: the requires-derived copy-paste hint, deterministic for --check.
fn suggest_text(a: &Authz) -> String {
    let mut lines = vec![format!("# authorize_skill_requires: {}", a.flag)];
    if a.suggestions.is_empty() {
        lines.push("# (no enabled skill declares a `requires` command)".to_string());
    } else {
        lines.push(
            "# commands enabled skills require (paste an allow rule into permissions.md to grant):"
                .to_string(),
        );
        for s in &a.suggestions {
            let granted = if a.allow.contains(&s.command) {
                "  [granted]"
            } else {
                ""
            };
            lines.push(format!("#   {}  (from {}){}", s.command, s.tool, granted));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// JS pipelineBinArg: repeatable/comma-separated `--pipeline-bin` values, de-duped in order.
fn pipeline_bin_arg(args: &[String]) -> Vec<String> {
    let mut raw = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a == "--pipeline-bin" {
            if let Some(v) = args.get(i + 1) {
                raw.push(v.clone());
            }
        }
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in raw {
        for part in s.split(',') {
            let p = part.trim();
            if !p.is_empty() && seen.insert(p.to_string()) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// JS buildArtifacts bin list: (requiredToolNames ∪ pipelineTools) resolved to tools, filtered
/// to known ones, sorted by name.
fn bin_list<'a>(
    enable: &Enable,
    bin_tools: &'a HashMap<String, BinTool>,
    catalog: Option<&Path>,
    base: Option<&Path>,
    pipeline: &[String],
) -> Vec<&'a BinTool> {
    let mut names = required_tool_names(enable, catalog, base);
    names.extend(pipeline.iter().cloned());
    let mut seen = HashSet::new();
    let mut list: Vec<&BinTool> = names
        .into_iter()
        .filter(|n| seen.insert(n.clone()))
        .filter_map(|n| bin_tools.get(&n))
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// JS binInstallTsv: name<TAB>os-arch<TAB>url<TAB>sha256<TAB>archive<TAB>bin, one row per
/// (tool, platform). Platform keys sorted (JS default sort = UTF-16). Empty -> "".
fn bin_install_tsv(tools: &[&BinTool]) -> String {
    let mut rows = Vec::new();
    for t in tools {
        let bin = t.bin.clone().unwrap_or_else(|| t.name.clone());
        let archive = t.archive.clone().unwrap_or_else(|| "tar.gz".to_string());
        let pv = t.pinned_version.clone().unwrap_or_default();
        let url_tpl = t.url.clone().unwrap_or_default();
        let mut plats: Vec<&(String, Platform)> = t.platforms.iter().collect();
        plats.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
        for (plat, spec) in plats {
            let asset = spec.asset.clone().unwrap_or_default();
            let url = url_tpl
                .replace("${version}", &pv)
                .replace("${asset}", &asset);
            let sha = spec.sha256.clone().unwrap_or_default();
            rows.push(format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                t.name, plat, url, sha, archive, bin
            ));
        }
    }
    if rows.is_empty() {
        String::new()
    } else {
        let mut s = rows.join("\n");
        s.push('\n');
        s
    }
}

// ============================================================================
// skills axis (ADR 0008 Phase 1c) — port of sync.js buildSkillsBundle: stage the enabled
// skills' dirs (each rooted at <name>/), tar deterministically via the same `tar`, base64.
// ============================================================================

struct SkillsBundle {
    tar_b64: String,
    list: String,
}

/// Port of buildSkillsBundle. skills sorted by name; each staged under <name>/ then tarred
/// with GNU tar's determinism flags and base64'd (+ "\n"). skills.list = sorted names.
/// Empty enable set -> both "" (matches node). Staging modes mirror node so tar bytes match:
/// root 0700 (mkdtemp), dirs 0755 (mkdir default), files keep their source mode.
fn build_skills_bundle(
    enable: &Enable,
    catalog: Option<&Path>,
    base: Option<&Path>,
) -> SkillsBundle {
    let empty = SkillsBundle {
        tar_b64: String::new(),
        list: String::new(),
    };
    let mut skills: Vec<&EnableSkill> = enable.skills.iter().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name)); // localeCompare ≈ scalar order for skill names
    let staged = match make_stage_dir() {
        Some(d) => d,
        None => return empty,
    };
    let mut names: Vec<String> = Vec::new();
    for s in skills {
        let dir = match (s.source.as_str(), base) {
            ("personal", Some(b)) => b.join("skills").join(&s.name),
            _ => match catalog {
                Some(c) => c.join("skills").join(&s.name),
                None => continue,
            },
        };
        if !dir.exists() {
            eprintln!(
                "  skill {}: source missing ({}) — skipped",
                s.name,
                dir.display()
            );
            continue;
        }
        if copy_tree(&dir, &staged.join(&s.name)).is_ok() {
            names.push(s.name.clone());
        }
    }
    let tar_b64 = if names.is_empty() {
        String::new()
    } else {
        match Command::new("tar")
            .args([
                "--sort=name",
                "--mtime=UTC 2020-01-01",
                "--owner=0",
                "--group=0",
                "--numeric-owner",
                "-cf",
                "-",
                "-C",
            ])
            .arg(&staged)
            .arg(".")
            .output()
        {
            Ok(o) if o.status.success() => {
                let mut s = base64_encode(&o.stdout);
                s.push('\n');
                s
            }
            _ => String::new(),
        }
    };
    let _ = std::fs::remove_dir_all(&staged);
    let list = if names.is_empty() {
        String::new()
    } else {
        let mut s = names.join("\n");
        s.push('\n');
        s
    };
    SkillsBundle { tar_b64, list }
}

/// A fresh staging dir at mode 0700 (mkdtemp equivalent — tar records the `./` root with this
/// mode). Not cryptographically unique; pid+nanos suffices for a single render.
fn make_stage_dir() -> Option<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("capsync-skills-{}-{}", std::process::id(), nanos));
    std::fs::create_dir(&dir).ok()?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    Some(dir)
}

/// Recursive copy mirroring sync.js cpDir: dirs via create_dir_all (0755 under umask 022, ==
/// node mkdirSync default), files via fs::copy (preserves source mode == node copyFileSync).
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let e = entry?;
        let s = e.path();
        let d = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&s, &d)?;
        } else {
            std::fs::copy(&s, &d)?;
        }
    }
    Ok(())
}

/// Standard base64 (RFC 4648, `+/`, `=` padding, no line wrapping) — matches Node
/// Buffer.toString('base64').
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ============================================================================
// bin drift check (ADR 0008 Phase 1f-1) — port of install-bin.js platformKey / readLock /
// checkTool + sync.js checkTools. No network, no HOME writes; exit 1 on drift. --with-tools
// (install: fetch + verify + extract) lands in 1f-2.
// ============================================================================

/// JS platformKey(): `<os>-<arch>` with node's darwin→macos, x64→amd64, arm64 pass-through.
fn platform_key() -> String {
    let os = std::env::consts::OS; // "linux" / "macos" — matches node's platform spelling
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

struct BinLock {
    target: Option<String>,
    pinned_version: Option<String>,
    asset_sha256: Option<String>,
    bin_sha256: Option<String>,
}

fn json_get_str(j: &Json, key: &str) -> Option<String> {
    match json_get(j, key) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// JS readLock: parse ~/.agents-shared-capabilities/state/bin-lock/<name>.json (or None).
fn read_lock(home: &Path, name: &str) -> Option<BinLock> {
    let f = home
        .join(".agents-shared-capabilities")
        .join("state")
        .join("bin-lock")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&f).ok()?;
    let j = parse_json(&text).ok()?;
    Some(BinLock {
        target: json_get_str(&j, "target"),
        pinned_version: json_get_str(&j, "pinned-version"),
        asset_sha256: json_get_str(&j, "asset-sha256"),
        bin_sha256: json_get_str(&j, "bin-sha256"),
    })
}

/// Port of checkTool: verify installed state vs the registry. Returns (clean, STATUS, detail);
/// clean == ok|unsupported (no drift). Mirrors install-bin.js status words.
fn check_tool(tool: &BinTool, home: &Path) -> (bool, &'static str, String) {
    let pk = platform_key();
    let spec = tool
        .platforms
        .iter()
        .find(|(k, _)| *k == pk)
        .map(|(_, s)| s);
    let lock = read_lock(home, &tool.name);
    let Some(spec) = spec else {
        return (true, "UNSUPPORTED", format!("no asset for platform {pk}"));
    };
    let Some(lock) = lock else {
        return (false, "MISSING", "not installed (no lockfile)".to_string());
    };
    let Some(target) = lock.target.clone().filter(|t| !t.is_empty()) else {
        return (false, "MISSING", "not installed (no lockfile)".to_string());
    };
    if !Path::new(&target).exists() {
        return (false, "MISSING", format!("binary gone: {target}"));
    }
    if lock.pinned_version.as_deref() != tool.pinned_version.as_deref()
        || lock.asset_sha256.as_deref() != spec.sha256.as_deref()
    {
        return (
            false,
            "STALE",
            format!(
                "installed {} but registry pins {}",
                lock.pinned_version.clone().unwrap_or_default(),
                tool.pinned_version.clone().unwrap_or_default()
            ),
        );
    }
    match sha256_file(Path::new(&target)) {
        Some(h) if Some(h.as_str()) == lock.bin_sha256.as_deref() => (
            true,
            "OK",
            format!(
                "{} @ {target}",
                tool.pinned_version.clone().unwrap_or_default()
            ),
        ),
        _ => (
            false,
            "TAMPERED",
            format!("on-disk sha256 != lock ({target})"),
        ),
    }
}

/// `--check-tools`: verify each required tool's installed state for drift (exit 1). No network,
/// no HOME writes. Honors --capabilities (like --check). Port of sync.js checkTools().
fn check_tools(args: &[String]) -> Result<bool, String> {
    let cap_file = cap_file_from(args)?;
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "HOME not set".to_string())?);
    let catalog = resolve_catalog(args);
    let base = personal_base(&cap_file);
    let enable = parse_enable(&cap_file);
    let tools = load_bin_tools(catalog.as_deref(), base.as_deref());
    let names = required_tool_names(&enable, catalog.as_deref(), base.as_deref());
    let pk = platform_key();
    println!(
        "bin drift check (platform {pk}) — required: {}",
        if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        }
    );
    let mut drift = false;
    for nm in &names {
        match tools.get(nm) {
            None => {
                drift = true;
                eprintln!("  {nm}: DRIFT — not in bin/registry.yaml");
            }
            Some(t) => {
                let (clean, status, detail) = check_tool(t, &home);
                if !clean {
                    drift = true;
                }
                println!("  {nm}: {status} — {detail}");
            }
        }
    }
    println!(
        "{}",
        if drift {
            "\nbin drift detected — run `capsync sync --with-tools`"
        } else {
            "\nno bin drift — installed tools current"
        }
    );
    Ok(drift)
}

// ---- SHA-256 (RFC 6234), zero-crate — matches Node crypto sha256 hex digest. ----
#[rustfmt::skip]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, wi) in w.iter_mut().enumerate().take(16) {
            let b = i * 4;
            *wi = u32::from_be_bytes([chunk[b], chunk[b + 1], chunk[b + 2], chunk[b + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (hi, vi) in h.iter_mut().zip(v.iter()) {
            *hi = hi.wrapping_add(*vi);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

fn sha256_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|d| sha256_hex(&d))
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
        let a = build_authz(text, &Enable::default(), None, None);
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
        let a = build_authz(text, &Enable::default(), None, None);
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
            suggestions: Vec::new(),
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
            ..Default::default()
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

    const BIN_REG: &str = "\
tools:
  - name: demotool
    source: external:example
    pinned-version: v1.2.3
    bin: demotool
    archive: tar.gz
    url: \"https://example.com/demotool/${version}/${asset}\"
    platforms:
      linux-arm64:
        asset: demotool-linux-arm64.tar.gz
        sha256: bbbb
      linux-amd64:
        asset: demotool-linux-amd64.tar.gz
        sha256: aaaa
";

    #[test]
    fn bin_registry_and_tsv() {
        let tools = parse_bin_registry(BIN_REG);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "demotool");
        assert_eq!(tools[0].platforms.len(), 2);
        let refs: Vec<&BinTool> = tools.iter().collect();
        // platform keys re-sorted (amd64 before arm64); ${version}/${asset} substituted.
        assert_eq!(
            bin_install_tsv(&refs),
            "demotool\tlinux-amd64\thttps://example.com/demotool/v1.2.3/demotool-linux-amd64.tar.gz\taaaa\ttar.gz\tdemotool\n\
             demotool\tlinux-arm64\thttps://example.com/demotool/v1.2.3/demotool-linux-arm64.tar.gz\tbbbb\ttar.gz\tdemotool\n"
        );
        assert_eq!(bin_install_tsv(&[]), "");
    }

    #[test]
    fn requires_block_and_inline() {
        let block =
            "name: s\nrequires:\n  - name: cfdrop\n  - name: jq\n    min: \"1.7\"\nother: x\n";
        assert_eq!(parse_requires_names(block), vec!["cfdrop", "jq"]);
        let inline = "requires: [{ name: cfdrop }, { name: jq, min: 1.7 }]\n";
        assert_eq!(parse_requires_names(inline), vec!["cfdrop", "jq"]);
        assert_eq!(parse_requires_names("name: s\n"), Vec::<String>::new());
        assert_eq!(
            frontmatter_block("---\na: 1\nrequires:\n---\nbody"),
            "a: 1\nrequires:"
        );
    }

    #[test]
    fn authz_auto_derives_from_suggestions() {
        // auto mode: a requires-derived command becomes an allow; explicit does not.
        let sug = vec![Suggestion {
            tool: "demotool".into(),
            command: "demotool".into(),
        }];
        let a_auto = Authz {
            flag: "auto".into(),
            allow: js_sort(vec!["demotool".into()]),
            deny: vec![],
            suggestions: sug,
        };
        assert!(a_auto.allow.contains(&"demotool".to_string()));
        assert_eq!(
            suggest_text(&a_auto),
            "# authorize_skill_requires: auto\n\
             # commands enabled skills require (paste an allow rule into permissions.md to grant):\n\
             #   demotool  (from demotool)  [granted]\n"
        );
        let a_none = Authz {
            flag: "explicit".into(),
            allow: vec![],
            deny: vec![],
            suggestions: vec![],
        };
        assert_eq!(
            suggest_text(&a_none),
            "# authorize_skill_requires: explicit\n# (no enabled skill declares a `requires` command)\n"
        );
    }

    #[test]
    fn pipeline_bin_dedup() {
        let args: Vec<String> = ["--pipeline-bin", "jq,ripgrep", "--pipeline-bin", "jq"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(pipeline_bin_arg(&args), vec!["jq", "ripgrep"]);
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn platform_key_maps() {
        // whatever host we're on, the key is <os>-<arch> with node's arch spelling.
        let pk = platform_key();
        assert!(pk.contains('-'));
        assert!(!pk.contains("x86_64") && !pk.contains("aarch64"));
    }

    #[test]
    fn base64_matches_node() {
        // RFC 4648 vectors == Node Buffer.toString('base64'): padding + no wrapping.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0xff, 0xfe, 0xfd]), "//79"); // exercises + and /
    }
}
