# Frontmatter reference

Read when writing or changing a skill's frontmatter. Source of truth: <https://agentskills.io/specification>.

## Spec fields

| Field | Required | Constraints |
|-------|----------|-------------|
| `name` | yes | 1–64 chars; `a-z`, `0-9`, `-` only; no leading/trailing `-`, no `--`; **must equal the folder name** |
| `description` | yes | 1–1024 chars; what the skill does and when to use it |
| `license` | no | License name or a bundled license file (e.g. `Proprietary. LICENSE.txt has complete terms`) |
| `compatibility` | no | ≤500 chars; only for real environment requirements (target runtime, system packages, network access) |
| `metadata` | no | Map of string keys to string values for anything the spec doesn't define; use reasonably unique keys |
| `allowed-tools` | no | Experimental; space-separated pre-approved tools (e.g. `Bash(git:*) Read`); support varies by runtime |

Beyond the spec: some runtimes reject names containing the reserved words `claude` or `anthropic`; avoid them for portability.

## Extra top-level keys

The reference validator (`skills-ref validate`) rejects any top-level key outside the table above. Most runtimes ignore unknown keys, so tooling sometimes adds its own. To keep a skill portable, put extra data under `metadata` as strings; if a local toolchain requires its own top-level keys, keep them and accept that strict validation will flag them. `scripts/validate.sh` warns on unknown keys by default and errors with `--strict-spec`.

## Writing the description

The description is the only thing a runtime sees when deciding whether to load the skill — the body loads only after it triggers. So:

- Say both **what it does** and **when to use it**. Put the "when" here, not only in the body.
- Include the words and phrasings users actually say, in every language they use.
- Third person, concrete; avoid one-liners like "Helps with X".
- Add exclusions ("Not for …") only when they prevent likely misrouting; no long capability lists or catch-alls.
- Models tend to **under-trigger** skills, so lean slightly assertive — name situations where the user doesn't mention the skill but clearly needs it — without pulling in unrelated requests.

Good:

```yaml
description: Extracts text and tables from PDF files, fills PDF forms, and merges PDFs. Use when working with PDF documents or when the user mentions PDFs, forms, or document extraction.
```

Poor:

```yaml
description: Helps with PDFs.
```

## When updating

- Don't rename `name` or the folder — that makes it a different skill and breaks anything that enables it by name.
- Preserve existing `metadata` and runtime- or tool-specific fields; change only what's needed.
