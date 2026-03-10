//! 11 built-in skills shipped with Caboose.

use super::types::{ResponseFormat, Skill, SkillSource};

/// All built-in skills. Workflow skills (brainstorm, plan, debug, tdd, finish)
/// plus analysis skills (review, refactor, test, explain, optimize).
pub fn builtin_skills() -> Vec<Skill> {
    vec![
        Skill {
            name: "brainstorm".into(),
            description: "Diverge-then-converge design exploration".into(),
            template: BRAINSTORM_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "debug".into(),
            description: "Isolate faults by narrowing scope".into(),
            template: DEBUG_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "explain".into(),
            description: "Explain how code works".into(),
            template: EXPLAIN_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "finish".into(),
            description: "Audit branch before integration".into(),
            template: FINISH_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "handoff".into(),
            description: "Generate a structured handoff summary".into(),
            template: HANDOFF_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "optimize".into(),
            description: "Suggest performance optimizations".into(),
            template: OPTIMIZE_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "plan".into(),
            description: "Write a granular implementation plan".into(),
            template: PLAN_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Plan,
        },
        Skill {
            name: "refactor".into(),
            description: "Suggest refactoring improvements".into(),
            template: REFACTOR_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "review".into(),
            description: "Rule of Five code review".into(),
            template: REVIEW_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "tdd".into(),
            description: "Enforce RED-GREEN-REFACTOR cycle".into(),
            template: TDD_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
        Skill {
            name: "test".into(),
            description: "Generate test cases".into(),
            template: TEST_TEMPLATE.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        },
    ]
}

/// Convenience alias for the static list.
#[allow(dead_code)]
pub static BUILTIN_SKILLS: std::sync::LazyLock<Vec<Skill>> =
    std::sync::LazyLock::new(builtin_skills);

// --- Templates ---

const BRAINSTORM_TEMPLATE: &str = r#"# Brainstorm

Explore the design space before writing code. Three phases:

1. **Explore** — Generate 3-5 divergent approaches to: $ARGS
   - For each approach, state the core idea, key trade-offs, and what it optimizes for.
   - Include at least one unconventional option.

2. **Converge** — Compare approaches on: correctness, simplicity, performance, maintainability.
   - Recommend one approach with clear reasoning.

3. **Lock** — Write a concise decision record:
   - **Decision:** one sentence
   - **Rationale:** 2-3 sentences
   - **Trade-offs accepted:** what you're giving up
   - Save to `docs/plans/` if appropriate.
"#;

const DEBUG_TEMPLATE: &str = r#"# Debug

Isolate the fault systematically. Do NOT guess-and-check.

**Target:** $ARGS

1. **Reproduce** — Find the minimal input that triggers the bug. Run it, confirm the failure.
2. **Bisect** — Narrow the fault boundary. Comment out halves, add logging, binary-search for the broken line/module.
3. **Read** — Once narrowed to ~20 lines, read every line. Check types, nulls, off-by-ones, async ordering.
4. **Prove** — Write a failing test that captures the bug. Fix the code. Test passes. Run full suite to check for regressions.

Do not move to the next step until the current one is complete.
"#;

const EXPLAIN_TEMPLATE: &str = r#"# Explain

Explain how this code works: $ARGS

Cover:
- **High-level summary** — what it does in one paragraph
- **Key functions/methods** — purpose of each, how they relate
- **Data flow** — what goes in, what comes out, how data transforms
- **Design decisions** — why it's structured this way, what patterns it uses
- **Dependencies** — what it imports/relies on, why

Use bullet points. Be concise. Skip obvious things.
"#;

const FINISH_TEMPLATE: &str = r#"# Finish

Audit the current branch before integration. $ARGS

**Quality gates** (run each, report pass/fail):
1. Build: does it compile without warnings?
2. Tests: do all tests pass?
3. Lint: does it pass linting?
4. Uncommitted: is the working tree clean?

**Diff audit** (review `git diff main...HEAD`):
1. **Scope check** — are all changes related to the stated goal?
2. **Regression sniff** — any deleted tests, weakened assertions, or commented-out code?
3. **Residue check** — any TODOs, debug prints, or temporary hacks left behind?

Report findings. Do NOT push or merge without explicit user confirmation.
"#;

const OPTIMIZE_TEMPLATE: &str = r#"# Optimize

Suggest performance optimizations for: $ARGS

For each suggestion:
- **What:** the specific change
- **Why:** what bottleneck it addresses (allocations, algorithmic complexity, I/O, caching, parallelization, memory layout)
- **Impact:** High / Medium / Low
- **Trade-off:** what you lose (complexity, readability, memory)

Prioritize high-impact, low-complexity changes. Show before/after code where helpful.
"#;

const PLAN_TEMPLATE: &str = r#"# Plan

Write a granular implementation plan for: $ARGS

**Rules:**
- Each step is ONE action (2-5 minutes): write a test, run it, implement, run again, commit
- Include exact file paths for every file touched
- Include complete code (not "add validation" — show the actual code)
- Include exact test commands with expected output
- TDD: write the failing test FIRST, then implement
- Commit after each logical unit

**Format per step:**
1. What to do (one sentence)
2. File(s) to touch
3. Code to write
4. Command to run + expected result
"#;

const REFACTOR_TEMPLATE: &str = r#"# Refactor

Suggest refactoring improvements for: $ARGS

Focus areas:
- **DRY** — duplicated logic that should be extracted
- **Naming** — unclear variable/function/type names
- **Simplification** — overly complex logic that can be flattened
- **Extraction** — functions doing too many things
- **Type safety** — stringly-typed code that should use enums/structs

For each suggestion, show before/after code. Explain why the refactoring improves the code. Do NOT change behavior — refactoring preserves functionality.
"#;

const REVIEW_TEMPLATE: &str = r#"# Code Review

Review the following code with five iterative passes: $ARGS

**Pass 1 — Exploration:** Read through the code. Understand what it does, how it's structured, what patterns it uses. Note first impressions.

**Pass 2 — Correctness:** Check for bugs, logic errors, edge cases, null/undefined handling, off-by-one errors, race conditions, resource leaks.

**Pass 3 — Clarity:** Evaluate naming, structure, comments. Can you understand each function in <30 seconds? Flag anything confusing.

**Pass 4 — Edge Cases:** Think about what happens with empty input, huge input, concurrent access, network failures, malformed data.

**Pass 5 — Excellence:** Step back. Is this the right approach? Are there simpler alternatives? Any architectural concerns?

**Report findings as:**
- CRITICAL: Must fix before merge (bugs, security issues)
- WARNING: Should fix (code smells, potential issues)
- SUGGESTION: Nice to have (style, minor improvements)
"#;

const TDD_TEMPLATE: &str = r#"# TDD

Use strict RED-GREEN-REFACTOR test-driven development for: $ARGS

**Cycle:**
1. **RED** — Write ONE failing test. Run it. Confirm it fails with the expected error.
2. **GREEN** — Write the MINIMUM code to make the test pass. No more. Run the test. Confirm it passes.
3. **REFACTOR** — Clean up. Remove duplication, improve names, simplify. Run tests. Confirm they still pass.
4. **Repeat** — Pick the next behavior to test. Go to RED.

**Rules:**
- One test at a time. Never write two tests before making the first one pass.
- Run tests after EVERY change (red, green, AND refactor).
- In GREEN, write the stupidest code that works. Improve it in REFACTOR.
- Commit after each complete RED-GREEN-REFACTOR cycle.
"#;

const TEST_TEMPLATE: &str = r#"# Generate Tests

Generate test cases for: $ARGS

Cover:
- **Happy paths** — normal expected inputs and outputs
- **Edge cases** — empty input, boundary values, single elements, max sizes
- **Error conditions** — invalid input, missing data, network failures, permission errors

Use the project's existing test framework and conventions. Mock external dependencies. Write complete, runnable test code — not pseudocode.
"#;

const HANDOFF_TEMPLATE: &str = r#"# Handoff

Generate a structured handoff summary for the current conversation. Include:

1. **What was accomplished** — key actions taken, features built, bugs fixed
2. **What's left to do** — remaining tasks, known issues, next steps
3. **Key decisions** — architectural choices, trade-offs made, rationale
4. **Context for next session** — important file paths, patterns established, gotchas discovered

$ARGS

Format as a clean markdown document that can be pasted into a new session to resume work.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_eleven_builtin_skills() {
        assert_eq!(BUILTIN_SKILLS.len(), 11);
    }

    #[test]
    fn all_builtins_have_nonempty_templates() {
        for skill in &*BUILTIN_SKILLS {
            assert!(!skill.name.is_empty(), "skill name empty");
            assert!(
                !skill.description.is_empty(),
                "desc empty for {}",
                skill.name
            );
            assert!(
                !skill.template.is_empty(),
                "template empty for {}",
                skill.name
            );
            assert_eq!(skill.source, SkillSource::Builtin);
        }
    }

    #[test]
    fn all_builtins_contain_args_placeholder() {
        for skill in &*BUILTIN_SKILLS {
            assert!(
                skill.template.contains("$ARGS"),
                "skill {} missing $ARGS placeholder",
                skill.name
            );
        }
    }

    #[test]
    fn builtin_names_are_lowercase() {
        for skill in &*BUILTIN_SKILLS {
            assert_eq!(skill.name, skill.name.to_lowercase());
        }
    }

    #[test]
    fn workflow_skills_have_correct_formats() {
        let plan_skill = BUILTIN_SKILLS.iter().find(|s| s.name == "plan").unwrap();
        assert_eq!(plan_skill.response_format, ResponseFormat::Plan);

        let brainstorm = BUILTIN_SKILLS
            .iter()
            .find(|s| s.name == "brainstorm")
            .unwrap();
        assert_eq!(brainstorm.response_format, ResponseFormat::Prose);
    }
}
