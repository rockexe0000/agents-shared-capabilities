# Evaluation and iteration

Read when a skill is complex or risky enough that behavioral evidence is worth the effort. Small edits or simple skills usually need only `scripts/validate.sh` plus a careful reread.

## Contents

1. Behavioral tests (with vs. without the skill)
2. Grading
3. Triggering tests (is the description accurate?)
4. Improving the skill
5. Environments without subagents

## 1. Behavioral tests

1. Write 2–3 test prompts a real user would send, with concrete detail (file names, columns, situation). Show them to the user first.
2. Run in an isolated scratch workspace so artifacts don't land in the repository, e.g. `<scratch>/<skill-name>-eval/iteration-<N>/<eval-name>/`. If the runtime sandboxes file writes to its workspace, pick a scratch directory inside that workspace rather than a system temp dir.
3. Run each prompt twice, in parallel when possible:
   - **with_skill**: give the executor the skill path, the prompt, and an output location.
   - **baseline**: for a new skill, no skill; for a revision, snapshot the old version first (`cp -r`) and point at the snapshot.
4. Give the executor only the realistic request, the skill, and the raw inputs it needs. Don't reveal the expected answer, the bug you suspect, or the conclusion you hope for — otherwise you're testing your hints, not the skill.

```text
Use the skill at <path/to/skill> to complete this request:
<realistic user prompt>
Save outputs to <scratch>/<skill-name>-eval/iteration-1/<eval-name>/with_skill/
```

Use fictional test data only — never real personal data, credentials, or internal URLs.

## 2. Grading

- Objectively checkable results (generated files, format conversions, fixed workflows) → write assertions with self-explanatory names. Check with a script when you can; it's reusable next iteration.
- Subjective results (style, design) → leave to human review instead of forcing assertions.
- Test observable behavior or meaningful invariants, not generated wording, headings, or regex shapes.
- Read the process (transcripts, logs), not just the final outputs: is the skill causing wasted steps? Did every run re-write the same helper? (→ bundle it into `scripts/`.)
- Watch for assertions that pass both with and without the skill — they don't discriminate.

## 3. Triggering tests

The description decides whether the skill loads at all, so it deserves its own test:

1. Write ~20 queries, roughly half should-trigger and half should-not-trigger.
2. Should-trigger: varied phrasings of the same intent (formal, casual, typos, not naming the skill but clearly needing it), and cases where this skill competes with another and should win.
3. Should-not-trigger: **near misses** — shared keywords or adjacent domains — are the valuable ones. Obviously unrelated queries test nothing.
4. Make queries substantive. Runtimes usually don't consult a skill for something they can do in one easy step, so trivial queries won't trigger regardless of the description.
5. Have the user review the query set before running it. When tuning the description, hold some queries out so you don't overfit.

## 4. Improving the skill

- **Generalize; don't overfit.** The skill will meet far more prompts than these few test cases. For stubborn problems, try a different framing or approach instead of adding case-specific MUSTs.
- **Stay lean.** Remove sections that aren't pulling their weight or that send the executor down unproductive paths.
- **Explain the why.** Before writing caps-lock ALWAYS/NEVER, try stating the reason.
- **Bundle repeated work.** If several runs independently wrote similar helpers, turn that into a script in `scripts/` and point to it from `SKILL.md`.
- Rerun all test cases (including baseline) into `iteration-<N+1>/`. Stop when the user is satisfied, feedback is empty, or progress stalls.

## 5. Environments without subagents

Run each test yourself by reading `SKILL.md` and following it, one at a time. This is less independent (you wrote the skill and know the context), so skip baselines and quantitative comparison and rely on the user's qualitative feedback.

Ask before running evaluations that cost substantial time or money, need additional authorization, or touch a production system.
