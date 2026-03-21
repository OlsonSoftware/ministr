# /init

Interactive project interview — tailors the magistr scaffold to your specific project.

## Instructions

This skill reads the auto-detected configuration from `.magistr/project.json`, confirms it with the user, then rewrites project files with project-specific content.

## Steps

1. **Load current config** — Read `.magistr/project.json` to see what `magistr init` auto-detected.

2. **Present detected stack** — Show the user what was detected:
   ```
   Detected: [language] + [framework] ([package_manager])
   Test: [test_framework] | Lint: [linter] | Format: [formatter]
   ```

3. **Confirm or correct** — Ask the user if the detection is correct. If not, update project.json.

4. **Interview** — Ask up to 6 targeted questions (skip any already answered by detection or manifest):

   a. **Project description** — "What does this project do? (1-2 sentences)"
      Skip if the manifest already has a description.

   b. **Project type** — "What kind of project is this?"
      Options: `lib` (library/package), `app` (web application), `api` (backend API), `cli` (command-line tool), `monorepo` (multiple packages)

   c. **Design spec** — "Do you have a design spec or roadmap?"
      Options: "Point me to an existing file", "Generate a starter template", "Skip for now"
      If existing file, update `spec.file` in project.json.

   d. **Reference codebases** — "Any reference codebases the agent should consult?"
      If yes, add paths to `reference_repos` in project.json and suggest magistr-scope indexing.

   e. **Conventions** — "Any project-specific conventions beyond standard [language] style?"
      E.g., naming patterns, architectural rules, forbidden patterns.

   f. **Never do** — "Anything the agent should NEVER do in this project?"
      E.g., "never modify the database schema directly", "never use class components"

5. **Research** — Use WebSearch to find current best practices for the detected stack:
   - Idiomatic patterns for the language/framework
   - Recommended project structure
   - Common testing patterns
   - Popular linting rules

6. **Rewrite files** based on interview answers + research:

   a. **`.magistr/project.json`** — Update with confirmed/corrected values, description, type

   b. **`CLAUDE.md`** — Rewrite with:
      - Project identity and description
      - Correct quick-start commands
      - Project-specific workflow notes
      - Reference repo instructions (if any)
      - "Never do" rules (if any)

   c. **`.claude/rules/conventions.md`** — Rewrite with:
      - Language-specific naming conventions
      - Framework idioms and best practices
      - Project-specific patterns from interview
      - Anti-patterns to avoid

   d. **`.claude/settings.local.json`** — Update allowed tools based on confirmed stack

7. **Print summary** of everything that was configured:
   ```
   ## magistr configured

   - Project: [name] ([type])
   - Stack: [language] + [framework]
   - Quality gate: [check] && [test] && [lint]
   - Spec: [spec file]
   - Conventions: [summary of key conventions]

   Ready to use /next, /validate, /roadmap
   ```
