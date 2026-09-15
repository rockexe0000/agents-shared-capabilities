# tools-rs — capability tooling, Rust port (ADR 0008)

`capsync`: a single-file, runtime-portable reimplementation of `tools/` (the capability
projection tooling). ADR 0008 unifies the node + POSIX-sh backends into one static binary.
During migration `tools-rs/` runs **parallel** to `tools/` — node/sh stays canonical until
per-axis byte-for-byte parity is proven, then node is removed (Phase 4).

Decision WHY / trade-offs: `agents-cold-memory:shared/adr/0008-tooling-rust-unification`.
Work-list: the matching handoff (`…/handoffs/discord-1548114275158200351-tooling-rust-unification`).

## Scope (current)

`capsync` mirrors `tools/sync.js` at byte-for-byte parity, all JSON emitted as
`JSON.stringify(x, null, 2) + "\n"` via a hand-rolled serializer:

| command | what it does |
|---------|--------------|
| `--render <dir> [--capabilities <f>] [--catalog <d>]` | write every artifact off-pod: `authz-*.json` + `authz-suggest.txt`, `openab-agent-mcp.json` (facade) + `runtime-mcp.json` (direct), `bin-install.tsv`, `skills.tar.b64` + `skills.list` |
| `--check <dir> …` | re-render + diff committed artifacts; exit 1 on drift |
| `sync [--with-tools] [--catalog <d>]` | live projection into `$HOME`: skills symlinks, MCP merge into each runtime config (claude-code/codex/antigravity/opencode + oab-facade), hooks (claude/antigravity); `--with-tools` also fetches+verifies the pinned bin CLIs enabled skills `require` |
| `--check-tools …` | verify installed bin tools vs the registry (drift → exit 1); no network |

`--catalog <dir>` points at a local checkout of this catalog repo (`sync.js` infers it from
`__dirname`; a standalone binary can't, so it's an explicit flag — falls back to an exe-relative
guess). Ports `tools/lib/{parse,authz,install-bin}.js`, `tools/sync.js`, `tools/projectors/*`.

## Build / test / parity

```sh
cd tools-rs
cargo test                    # unit tests (parsers / json / sha256 / base64 / shapes)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
./parity.sh                   # build rust + run node, diff both vs committed golden
./parity.sh --node-only       # skip the rust half (env without a C linker)
./parity.sh --update-golden   # regenerate golden from node (maintenance)
```

The gate renders every fixture under `tests/fixtures/<case>/` three ways — node, rust, and the
committed `tests/golden/<case>/` — and fails on any diff, so a regression in **either** side is
caught. It also exercises the stateful paths in isolated `$HOME`s (live `sync` skills/MCP/hooks,
`--check`, `--check-tools`, `--with-tools` via a `file://` fake asset) comparing node vs rust.
CI runs the full gate on `ubuntu-latest` (`.github/workflows/tooling-rs-parity.yml`).

> A linker-less environment (no `cc`/`gcc`) can still run `cargo check`, `fmt`, `clippy`, and
> `./parity.sh --node-only`, but not `cargo test`/`build`/full parity (all require linking). The
> executing rust-vs-node byte comparison runs in CI.

## Supply chain (ADR 0008 Decision 2 / ADR 0006 template)

`capsync` is a managed supply-chain object: reproducible build, pinned + verifiable.

- **Toolchain pinned** to an exact version in `rust-toolchain.toml` (not floating `stable`);
  bumping is a deliberate PR. `Cargo.lock` is committed; release builds use `--locked`.
- **Release** (`.github/workflows/tooling-rs-release.yml`, on a `tooling-rs-v*` tag):
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
