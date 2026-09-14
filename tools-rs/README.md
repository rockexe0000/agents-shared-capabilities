# tools-rs — capability tooling, Rust port (ADR 0008)

Single-file, runtime-portable reimplementation of `tools/` (the capability
projection tooling). This is the **Phase 0 spike** of ADR 0008 (tooling Rust
unification + on-pod render): it exists to de-risk the rewrite, not to replace
`tools/` yet. Node/sh stays canonical until per-axis byte-for-byte parity is proven.

Decision WHY / trade-offs: `agents-cold-memory:shared/adr/0008-tooling-rust-unification`.
Implementation work-list: the matching handoff (`…/handoffs/discord-1548114275158200351-tooling-rust-unification`).

## Scope of the spike

Implements **`--render` for the authorization axis only** (閘 2, ADR 0007):

```
permissions.md  →  {allow, deny}  →  per-runtime tokens  →  authz-<runtime>.json
```

Target: **byte-for-byte identical** to `tools/sync.js --render`'s
`authz-antigravity.json` and `authz-claude-code.json`. The code is a deliberate
port of `tools/lib/parse.js` (`parsePermissions` / `strip` / `stripComment`) and
`tools/lib/authz.js` (`buildAuthz` / `mapAuthz`), down to emitting
`JSON.stringify(obj, null, 2) + "\n"` via a hand-rolled serializer.

**Intentionally NOT in the spike** (Phase 1 territory — see the handoff):

- `authz-suggest.txt` and `authorize_skill_requires: auto` grant-derivation. Both
  need the enable-list + bin-tools registry (`loadBinTools` / `requiredToolNames`).
  The flag is parsed and honored, but no `requires`-derived suggestions are
  synthesized. This does not change the two JSON files when no skill is enabled
  (see the `auto-no-skills` fixture).
- The other axes: mcp (facade/direct), bin (TSV), skills (tar), hooks.
- `--check`, `--with-tools`, `--check-tools`, cross-compile + attest + sha256 pin.

## Phase 0 toolchain decisions (ADR 0008 Decision 1–2)

- **Zero external crates.** The authz artifact is a fixed-shape object of string
  arrays; a hand-rolled printer that mirrors `JSON.stringify(x, null, 2)` is smaller
  and safer for parity than pulling `serde_json` (whose escaping / key-order we'd
  have to re-verify against V8 anyway). Revisit only when an axis needs real JSON
  *parsing*.
- **Toolchain pinned** via `rust-toolchain.toml` (`channel = "stable"`). Phase 1
  pins the exact version + cross-compile targets (linux amd64/arm64 + macos) and
  wires attest + sha256 pinning per ADR 0006.
- **Layout: `tools-rs/` sits beside `tools/`,** both live during migration; the
  parity gate protects the switch.

## Build / test / parity

```sh
cd tools-rs
cargo test                 # unit tests (parse / build / json shape)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
./parity.sh                # build rust + run node, diff both vs committed golden
./parity.sh --node-only    # skip the rust half (env without a C linker)
./parity.sh --update-golden  # regenerate golden from node (maintenance)
```

The parity gate renders every fixture under `tests/fixtures/<case>/` three ways —
node, rust, and the committed `tests/golden/<case>/` — and fails on any diff, so a
regression in **either** implementation is caught. CI runs the full gate on
`ubuntu-latest` (`.github/workflows/tooling-rs-parity.yml`); that runner has the C
toolchain the Rust link step needs.

> Note: a linker-less environment (no `cc`/`gcc`, no dev libc) can still run
> `cargo check`, `cargo fmt`, `cargo clippy`, and `./parity.sh --node-only`, but
> not `cargo test` / `cargo build` / the full parity (all require linking). The
> spike was authored under exactly that constraint; the executing byte-for-byte
> comparison of both sides runs in CI.

## Fixtures

`tests/fixtures/<case>/agent/{capabilities,permissions}.md` drive the gate (the
subdir is `agent/`, not `personal/`, because the repo `.gitignore` excludes
`**/personal/` for real agent namespaces):

| case | exercises |
|------|-----------|
| `single-allow`    | the real deployed shape (allow `cfdrop` only) |
| `deny-override`   | deny removes a command from allow; `ask` left unprojected; sort |
| `quotes-comments` | double/single-quote stripping, inline `#` comment stripping, dedup |
| `spaces`          | command token with a space (JSON + sort) |
| `empty`           | no `run command` rules → `{"allow":[],"deny":[]}` |
| `auto-no-skills`  | `auto` flag with no skills → JSON unchanged vs explicit |
