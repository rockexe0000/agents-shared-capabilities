---
name: skill-creator
description: Create a new Agent Skill or revise an existing one (SKILL.md frontmatter, instructions, scripts/, references/, assets/) following the Agent Skills specification, then validate and optionally test it. Use when the user wants to make, write, scaffold, update, refactor, validate, or test a skill, improve a skill's description or triggering, or turn a workflow from the conversation into a reusable skill. Not for system prompts, memory/notes files, MCP servers, or hooks.
license: MIT
metadata:
  short-description: Create, update, and validate Agent Skills
  based-on: "anthropics/skills skill-creator; openai/codex sample skill-creator; agentskills.io specification"
---

# Skill Creator

Create or revise skills that help a future agent make better decisions on a specific kind of task, without constraining unrelated work. Applies to any runtime that supports Agent Skills.

## Principles

- **Assume the agent is already capable.** Include only information that changes its decisions or improves its output. Cut generic advice, repeated instructions, speculative edge cases, and examples that don't materially clarify anything.
- **Preserve the user's intent and scope.** A skill supports the requested task; it should not swap the user's chosen tools, expand the assignment, touch unrelated configuration, or imply permission for extra external actions. Don't promote a single example, past failure, or personal preference into a universal rule. Approval to do a task doesn't widen its scope or permissions; for retrying or externally mutating workflows, define a stopping condition proportional to the risk.
- **Match specificity to risk.** For open-ended work, describe the outcome and the decision criteria and let the agent choose an approach. For work with a preferred shape, give examples or configurable scripts. Reserve fixed steps, deterministic scripts, and absolute language for cases where deviation causes a concrete problem (correctness, safety, permissions, fragile workflows).
- **Explain the why.** If you catch yourself writing ALWAYS or NEVER in caps, try explaining the reason instead — a model that understands the purpose handles unanticipated cases better than one following a rule.
- **Keep discovery cheap and precise.** `name` and `description` are visible before the skill loads. State what it does and when to use it; add exclusions only when they prevent likely misrouting. Avoid exhaustive capability lists and catch-alls.
- **Disclose progressively.** Keep shared purpose, essential constraints, and routing in `SKILL.md`. Move substantial context-specific material to `references/` and say when to read each file. A simple self-contained skill needs no router or extra files.
- **No surprises.** A skill's contents should not surprise the user if described to them. Don't write malware, or skills designed for unauthorized access or data exfiltration.

## Anatomy

```text
skill-name/
├── SKILL.md        required: YAML frontmatter (name + description) + Markdown instructions
├── scripts/        optional: deterministic, repeated logic (run it; no need to load it)
├── references/     optional: docs read only when relevant (schemas, APIs, domain rules, detailed procedures)
└── assets/         optional: files used in the output (templates, icons, fonts, boilerplate)
```

Create only the directories and files the task actually needs. Skip README, install guides, changelogs, and duplicate quick references. Keep references one level deep from `SKILL.md`, linked by relative path; give references over ~300 lines a table of contents or searchable keywords.

## Workflow

Scale the process to the request: a complex new skill may need every step; a small edit may need only a focused change and validation.

### 1. Capture intent

If the conversation already contains the workflow to capture, extract it first: tools used, step order, corrections the user made, input/output formats. Establish:

1. What should the skill enable the agent to do?
2. Which requests or contexts should trigger it?
3. What does the output look like?
4. Is testing worthwhile? Objectively checkable outputs (file transforms, data extraction, fixed workflows) benefit from it; subjective ones (writing style, design) usually don't.

Ask only when the missing information matters and can't reasonably be inferred. If the user has already explained the task clearly, proceed.

### 2. Choose the location

Respect a user-specified location. Otherwise use the runtime's documented skills directory — personal (e.g. `~/.claude/skills/` for Claude Code, `$CODEX_HOME/skills/` for Codex) or project-level if the skill belongs to one repo.

If skills in this environment are managed from a source repository and copied or linked into runtimes by a sync tool, edit the source, not the projected copy — the copy gets overwritten on the next sync.

Check that a skill is the right container: a repeatable procedure with steps, scripts, or templates is a skill; standing behavioral rules, facts, or decision records belong in instruction/memory files instead.

### 3. Plan reusable resources

Work backward from realistic requests, and add resources only when the benefit is concrete:

- The same transformation would be rewritten each time → `scripts/`. Prefer dependencies that are reliably present in the target environment (POSIX sh runs almost anywhere); document anything else the script needs.
- A schema, API, or rule set would be rediscovered each time → `references/<topic>.md`.
- Templates or files that go into the output → `assets/`.
- Several mutually exclusive modes (e.g. different cloud providers) → one reference per mode; `SKILL.md` holds only the selection criteria so the agent reads just the relevant one.

### 4. Write the frontmatter

Minimal:

```yaml
---
name: my-skill
description: <What it does>. Use when <triggers: phrases users say, contexts, keywords>.
---
```

Naming: lowercase letters, digits, single hyphens, ≤64 characters, folder name = `name`. Prefer short, action-oriented names; prefix with a tool or domain when it helps discovery. For field limits, optional fields, and how to write a description that triggers well, read [references/frontmatter.md](references/frontmatter.md).

### 5. Write the body

Write what another agent needs to perform the task: the desired outcome, non-obvious context, real constraints, and relevant references or tools. Use the imperative. Give a template when the output format must be exact, and an example or two when style matters. Don't prescribe structure, process, or step counts the task doesn't require. Keep `SKILL.md` under ~500 lines; split detail into `references/` as you approach that.

Use only fictional data in examples and test fixtures. Write credentials as placeholders such as `${API_KEY}` or `<YOUR_TOKEN>`, never real values.

Then reread with fresh eyes: does each section change the agent's decisions? If not, cut it.

### 6. Validate

```sh
scripts/validate.sh <path/to/skill>                # spec checks; unknown top-level keys only warn
scripts/validate.sh --strict-spec <path/to/skill>  # unknown keys are errors (matches skills-ref validate)
```

Paths are relative to this skill's directory. It checks frontmatter, naming, length limits, leftover placeholders, relative links, and script permissions. Also run any linter the destination repository uses, and actually execute new or changed scripts.

Passing validation means the format is right, not that the skill makes good decisions. Also confirm the description discriminates, the instructions preserve user intent, and references are discoverable.

### 7. Test (when warranted)

For skills complex or risky enough that behavioral evidence adds real confidence, run with/without-skill comparisons and triggering tests — read [references/evaluation.md](references/evaluation.md). Ordinary creation or small edits don't need this.

### 8. Deliver

Summarize what the skill does, what triggers it, and the validation results. Installing or enabling the skill for a runtime, and granting permission for the commands it runs, are separate steps — don't change those configurations unless the user asks.

## Updating an existing skill

- Read the whole skill first, including callers of scripts and references; confirm a resource is unused before removing it.
- Keep `name`, the folder name, and existing frontmatter fields; change only what's needed.
- Make narrow corrections based on real usage or observed failures rather than accumulating universal rules from single cases.
- For a skill copied from an upstream source, keep edits minimal and note where and why it diverges from upstream.
