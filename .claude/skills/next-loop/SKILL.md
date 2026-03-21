# /next-loop

Activate the auto-loop and start implementing spec items continuously.

## Instructions

This skill activates the stop hook loop and immediately kicks off the first `/next` iteration. After each successful `/next` cycle (commit landed), the stop hook automatically chains another `/next`.

Read `.magistr/project.json` for `state_prefix` to derive state file paths.

**For fresh context per iteration, use the autonomous runner instead:**
```bash
bash run-autonomous.sh
```
Each iteration runs in a fresh agent session — zero context degradation.

The loop stops when:
- All spec items are checked off
- Max iterations reached (default 10, override with `MAGISTR_MAX_ITERATIONS`)
- A `/next` cycle fails to produce a commit
- The user interrupts (Ctrl+C / Escape)
- State file removed (`rm /tmp/{state_prefix}-next-loop`)

## Steps

1. Read `.magistr/project.json` to get `state_prefix`.

2. Clean up any leftover state from previous runs:
   ```bash
   rm -f .magistr/current-plan.md .magistr/current-task.md
   ```

3. Activate the loop by writing the state file:
   ```bash
   echo "0 none" > /tmp/{state_prefix}-next-loop
   ```

4. Immediately invoke `/next` to start the first iteration.
