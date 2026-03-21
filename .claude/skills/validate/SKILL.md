# /validate

Run the full quality gate for the project.

## Instructions

Read `.magistr/project.json` to get the quality gate commands. Execute all three checks in sequence. Report results clearly — pass/fail for each gate with any error details.

## Steps

1. Read `.magistr/project.json` to get `quality_gate.check`, `quality_gate.test`, and `quality_gate.lint`.

2. Run each command in sequence:
   - **Check** — compilation/type check
   - **Test** — all tests
   - **Lint** — linter

3. Report summary:
   ```
   ## Quality Gate
   - check: PASS/FAIL
   - test: PASS/FAIL  (N tests)
   - lint: PASS/FAIL

   [any actionable errors]
   ```
