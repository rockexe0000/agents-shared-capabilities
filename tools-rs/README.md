# tools-rs — capability tooling, Rust port (ADR 0008)

`capsync`: a single-file, runtime-portable reimplementation of the capability projection
tooling. ADR 0008 unified the former node + POSIX-sh backends into this one static binary.
The migration is complete: `capsync` is now the sole implementation — the node `tools/`
reference backend and the sh pod-appliers were retired once per-axis byte parity held.

Decision WHY / trade-offs (ADR 0008) and the migration work-list (handoff) are indexed in
[`../docs/design-notes.md`](../docs/design-notes.md).

## Scope (current)

All JSON is emitted as `JSON.stringify(x, null, 2) + "\n"` (the shape the retired node backend
produced, frozen into the committed golden) via a hand-rolled serializer:

| command | what it does |
|---------|--------------|
| `--render <dir> [--capabilities <f>] [--catalog <d>]` | write every artifact off-pod: `authz-*.json` + `authz-suggest.txt`, `openab-agent-mcp.json` (facade) + `runtime-mcp.json` (direct), `bin-install.tsv`, `skills.tar.b64` + `skills.list` |
| `--check <dir> …` | re-render + diff committed artifacts; exit 1 on drift |
| `sync [--with-tools] [--catalog <d>]` | live projection into `$HOME`: skills symlinks, MCP merge into each runtime config (claude-code/codex/antigravity/opencode + oab-facade), hooks (claude/antigravity); `--with-tools` also fetches+verifies the pinned bin CLIs enabled skills `require` |
| `--check-tools …` | verify installed bin tools vs the registry (drift → exit 1); no network |
| `lint [--catalog <d>]` | validate the catalog (bin/skills/mcp/hooks registries: naming, uniqueness, secret hygiene, supply-chain pins, `requires` floors); errors → exit 1 |

`--catalog <dir>` points at a local checkout of this catalog repo. A standalone binary can't
infer it from its own path reliably, so it's an explicit flag — falling back to an exe-relative
guess (`<root>/tools-rs/target/<profile>/capsync` → repo root).

## Build / test / parity

```sh
cd tools-rs
cargo test                    # unit tests (parsers / json / sha256 / base64 / shapes)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
./parity.sh                   # build rust + diff render vs committed golden + stateful asserts
./parity.sh --update-golden   # regenerate golden from capsync (maintenance)
```

The gate renders every fixture under `tests/fixtures/<case>/` with `capsync` and diffs it against
the committed `tests/golden/<case>/`, so any render regression is caught. It also exercises the
stateful paths in isolated `$HOME`s (live `sync` skills/MCP/hooks, `--check`, `--check-tools`,
`--with-tools` via a `file://` fake asset, and `apply` of each axis) and asserts capsync's
behaviour directly. CI runs the full gate on `ubuntu-latest` (`.github/workflows/tools-rs-parity.yml`).

> A linker-less environment (no `cc`/`gcc`) can still run `cargo check`, `fmt`, and `clippy`, but
> not `cargo test`/`build` or `./parity.sh` (all require linking) — the executing byte comparison
> runs in CI.

## Supply chain (ADR 0008 Decision 2 / ADR 0006 template)

`capsync` is a managed supply-chain object: reproducible build, pinned + verifiable.

- **Toolchain pinned** to an exact version in `rust-toolchain.toml` (not floating `stable`);
  bumping is a deliberate PR. `Cargo.lock` is committed; release builds use `--locked`.
- **Release** (`.github/workflows/tools-rs-release.yml`, on a `v*` tag):
  cross-compiles static musl linux (amd64/arm64) + macos (arm64/x86_64), publishes each
  artifact's **sha256** + a **build-provenance attestation** (`actions/attest-build-provenance`).
- **Consumers** (Phase 2 ephemeral init) fetch a pinned release asset and verify its sha256
  before running it — the same governance ADR 0006 applies to external bins, applied to our own.

## Fixtures

`tests/fixtures/<case>/agent/{capabilities,permissions}.md` drive the gate (subdir `agent/`,
not `personal/`, because `.gitignore` excludes `**/personal/`). Some cases add `mcp/`, `bin/`,
`skills/` for the personal-namespace paths.

| case | exercises |
|------|-----------|
| `single-allow` / `deny-override` / `quotes-comments` / `spaces` / `empty` | authz: allow/deny/sort, quote+comment stripping, dedup, empty |
| `auto-no-skills` | `authorize_skill_requires: auto` with no skills |
| `mcp-basic`   | MCP render: facade stdio + direct http, `env:` → `${env:}` |
| `bin-basic`   | bin `requires` closure → `bin-install.tsv`; `auto` allow derivation + suggest |
| `skills-multi`| skills tar (sorted, nested dir) → `skills.tar.b64` + `skills.list` |

(The stateful `sync` / `--check-tools` / `--with-tools` scenarios are built inline by
`parity.sh` in isolated `$HOME`s rather than as committed fixtures.)
