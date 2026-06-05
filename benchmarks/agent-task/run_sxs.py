#!/usr/bin/env python3
"""Side-by-side agent benchmark SUITE: real coding agents solving the SAME
tasks WITH ministr vs WITHOUT it, across a difficulty ladder and multiple
models, with a deterministic pass/fail validator per task.

The runner is **headless Claude Code** (`claude -p --output-format json`) — a
real coding agent, already authenticated, that reports token usage. For every
(task × model) it runs two arms that differ ONLY in the discovery tool:

  arm A "ministr" : Read, Edit, Write, Bash  +  the ministr MCP server
                    (the corpus is pre-indexed with `ministr index`), NO grep.
  arm B "grep"    : Read, Edit, Write, Bash, Grep, Glob, and NO ministr.

Both run on a fresh copy of the task's fixture in a throwaway /tmp dir, must
make that task's test suite pass, and are measured on: validator pass/fail,
tokens, num_turns, total_cost_usd, wall-clock. Per-(task,model) tables plus an
aggregate scoreboard are printed and written to results.json.

TASKS live in tasks/<id>/ — each a self-contained:
    task.json   manifest  {id, difficulty, language, summary, validate, golden}
    task.md     the natural-language prompt handed to the agent
    fixture/    the broken project (committed)
    golden/     known-good file(s) for the no-LLM selftest

This calls a real LLM and spends real quota, so it is OPT-IN and never part of
any default gate. Cost is hard-capped per arm with `--max-budget-usd`.

Usage:
  python3 run_sxs.py --selftest                      # no LLM: every task is a valid red->green
  python3 run_sxs.py --dry-run --models haiku,sonnet # print the whole matrix, no spend
  python3 run_sxs.py --models haiku                  # run all tasks on haiku (cheap)
  python3 run_sxs.py --tasks easy-roman-numeral,hard-operator-precedence --models haiku,sonnet
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS_DIR = os.path.join(HERE, "tasks")

ARMS = {
    # arm key -> (label, allowed tools, uses_ministr)
    "a": ("ministr", "Read Edit Write Bash mcp__ministr__*", True),
    "b": ("grep", "Read Edit Write Bash Grep Glob", False),
}


def sh(cmd, cwd=None, timeout=None):
    p = subprocess.run(cmd, cwd=cwd, timeout=timeout,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return p.returncode, p.stdout, p.stderr


# --------------------------------------------------------------------------
# Tasks
# --------------------------------------------------------------------------

def load_tasks(only=None):
    """Load tasks/<id>/task.json manifests, sorted by difficulty then id."""
    order = {"easy": 0, "medium": 1, "hard": 2}
    tasks = []
    for entry in sorted(os.listdir(TASKS_DIR)):
        manifest = os.path.join(TASKS_DIR, entry, "task.json")
        if not os.path.isfile(manifest):
            continue
        spec = json.load(open(manifest))
        spec["_dir"] = os.path.join(TASKS_DIR, entry)
        if only and spec["id"] not in only:
            continue
        tasks.append(spec)
    tasks.sort(key=lambda t: (order.get(t.get("difficulty"), 9), t["id"]))
    return tasks


def make_workdir(task, tag):
    work = tempfile.mkdtemp(prefix=f"sxs-{tag}-")
    dst = os.path.join(work, "fixture")
    shutil.copytree(os.path.join(task["_dir"], "fixture"), dst)
    return dst


def validate(task, workdir):
    """Run the task's validator. Returns (passed, summary)."""
    code, out, err = sh(task["validate"], cwd=workdir, timeout=180)
    tail = (err or out).strip().splitlines()
    return code == 0, (tail[-1] if tail else f"exit {code}")


def apply_golden(task, workdir):
    for rel, gold in task.get("golden", {}).items():
        shutil.copyfile(os.path.join(task["_dir"], gold), os.path.join(workdir, rel))


def selftest(tasks):
    """No-LLM proof every task is a valid red->green (broken fails, golden passes)."""
    all_ok = True
    for task in tasks:
        broken = make_workdir(task, "selftest-broken")
        ok_broken, sb = validate(task, broken)
        fixed = make_workdir(task, "selftest-fixed")
        apply_golden(task, fixed)
        ok_fixed, sf = validate(task, fixed)
        shutil.rmtree(os.path.dirname(broken), ignore_errors=True)
        shutil.rmtree(os.path.dirname(fixed), ignore_errors=True)
        good = (not ok_broken) and ok_fixed
        all_ok = all_ok and good
        flag = "OK " if good else "BAD"
        print(f"  [{flag}] {task['id']:<26} broken={'fail' if not ok_broken else 'PASS?!'} "
              f"({sb})  golden={'pass' if ok_fixed else 'FAIL?!'} ({sf})")
    print("SELFTEST:", "OK — every task is a valid red->green" if all_ok else "BROKEN")
    return 0 if all_ok else 1


# --------------------------------------------------------------------------
# One (task, arm, model) run
# --------------------------------------------------------------------------

def build_claude_cmd(arm_key, workdir, prompt, model, budget, mcp_config_path):
    label, allowed, uses_ministr = ARMS[arm_key]
    cmd = [
        "claude", "-p", prompt,
        "--output-format", "json",
        "--model", model,
        "--permission-mode", "bypassPermissions",
        "--no-session-persistence",
        "--setting-sources", "user",
        "--strict-mcp-config",
        "--add-dir", workdir,
        "--max-budget-usd", str(budget),
        "--allowedTools", allowed,
    ]
    if uses_ministr:
        cmd += ["--mcp-config", mcp_config_path]
    return cmd


def run_one(task, arm_key, model, budget, keep, dry_run):
    label, allowed, uses_ministr = ARMS[arm_key]
    prompt = open(os.path.join(task["_dir"], "task.md")).read()
    workdir = make_workdir(task, f"{task['id']}-{label}-{model}")
    res = {"task": task["id"], "difficulty": task.get("difficulty"),
           "model": model, "arm": arm_key, "label": label,
           "uses_ministr": uses_ministr, "workdir": workdir}

    mcp_config_path = None
    if uses_ministr:
        mcp_config = {"mcpServers": {"ministr": {
            "command": "ministr", "args": ["serve", "--corpus", workdir]}}}
        mcp_config_path = os.path.join(os.path.dirname(workdir), "ministr-mcp.json")
        with open(mcp_config_path, "w") as fh:
            json.dump(mcp_config, fh)
        # Index from cwd=workdir with `--corpus .` (a cwd .ministr.toml would
        # otherwise override an absolute --corpus; the fixture has none).
        if dry_run:
            print(f"  [{task['id']}/{model}/{label}] INDEX (cwd={workdir}): ministr index --corpus .")
        else:
            t0 = time.time()
            code, out, err = sh(["ministr", "index", "--corpus", "."], cwd=workdir, timeout=900)
            res["index_secs"] = round(time.time() - t0, 1)
            if code != 0:
                res["error"] = f"ministr index failed (exit {code}): {err[-300:]}"
                return res

    cmd = build_claude_cmd(arm_key, workdir, prompt, model, budget, mcp_config_path)
    if dry_run:
        print(f"  [{task['id']}/{model}/{label}] RUN: claude -p <{task['id']} task.md> "
              f"--model {model} --allowedTools \"{allowed}\""
              + (f" --mcp-config {os.path.basename(mcp_config_path)} --strict-mcp-config" if uses_ministr else ""))
        res["dry_run"] = True
        if not keep:
            shutil.rmtree(os.path.dirname(workdir), ignore_errors=True)
        return res

    t0 = time.time()
    code, out, err = sh(cmd, cwd=workdir, timeout=1800)
    res["wall_secs"] = round(time.time() - t0, 1)
    try:
        payload = json.loads(out)
    except json.JSONDecodeError:
        res["error"] = f"could not parse claude JSON (exit {code}): {out[-300:]} {err[-200:]}"
        return res

    usage = payload.get("usage", {}) or {}
    res.update({
        "is_error": payload.get("is_error"),
        "num_turns": payload.get("num_turns"),
        "total_cost_usd": payload.get("total_cost_usd"),
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "cache_read_tokens": usage.get("cache_read_input_tokens"),
        "cache_creation_tokens": usage.get("cache_creation_input_tokens"),
        "result_text": (payload.get("result") or "")[:200],
    })
    passed, summary = validate(task, workdir)
    res["validator_passed"] = passed
    res["validator_summary"] = summary

    if not keep:
        shutil.rmtree(os.path.dirname(workdir), ignore_errors=True)
        if mcp_config_path and os.path.exists(mcp_config_path):
            os.remove(mcp_config_path)
    return res


# --------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------

def _fmt(v, kind):
    if v is None:
        return "—"
    if kind == "bool":
        return "YES" if v else "NO"
    if kind == "usd":
        return f"${v:.4f}"
    if kind == "int":
        return f"{v:,}"
    if kind == "sec":
        return f"{v:.0f}"
    return str(v)


def print_task_table(task_id, difficulty, model, a, b):
    cell = 12
    print(f"\n  {task_id}  ({difficulty}, {model})")
    print("  " + "metric".ljust(16) + "ministr".rjust(cell) + "grep".rjust(cell))
    rows = [
        ("solved?", "validator_passed", "bool"),
        ("turns", "num_turns", "int"),
        ("output tok", "output_tokens", "int"),
        ("cache-read tok", "cache_read_tokens", "int"),
        ("cost USD", "total_cost_usd", "usd"),
        ("wall s", "wall_secs", "sec"),
    ]
    for name, key, kind in rows:
        line = "  " + name.ljust(16)
        line += _fmt(a.get(key) if a else None, kind).rjust(cell)
        line += _fmt(b.get(key) if b else None, kind).rjust(cell)
        print(line)
    for arm in (a, b):
        if arm and arm.get("error"):
            print(f"    [{arm['label']}] ERROR: {arm['error']}")


def scoreboard(results, models, arms_run):
    """Aggregate across the matrix, headlining correctness then cost."""
    print("\n" + "=" * 60)
    print("AGGREGATE SCOREBOARD")
    print("=" * 60)
    for model in models:
        rows = [r for r in results if r["model"] == model and not r.get("dry_run")]
        if not rows:
            continue
        print(f"\nmodel: {model}")
        for arm_key in arms_run:
            label = ARMS[arm_key][0]
            arm_rows = [r for r in rows if r["arm"] == arm_key]
            solved = sum(1 for r in arm_rows if r.get("validator_passed"))
            cost = sum(r.get("total_cost_usd") or 0 for r in arm_rows)
            turns = sum(r.get("num_turns") or 0 for r in arm_rows)
            print(f"  {label:<8} solved {solved}/{len(arm_rows)}   "
                  f"total cost ${cost:.4f}   total turns {turns}")
        # head-to-head cost where BOTH solved the same task
        if set(arms_run) >= {"a", "b"}:
            ca = cb = 0.0
            both = 0
            for r in [x for x in rows if x["arm"] == "a"]:
                mate = next((x for x in rows if x["arm"] == "b" and x["task"] == r["task"]), None)
                if mate and r.get("validator_passed") and mate.get("validator_passed"):
                    ca += r.get("total_cost_usd") or 0
                    cb += mate.get("total_cost_usd") or 0
                    both += 1
            if both and cb:
                pct = round(100 * (1 - ca / cb))
                verb = "cheaper" if ca < cb else "more expensive"
                print(f"  head-to-head (both solved, {both} task(s)): "
                      f"ministr ${ca:.4f} vs grep ${cb:.4f} → ministr {abs(pct)}% {verb}")


# --------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description="Side-by-side agent benchmark suite (ministr vs grep).")
    ap.add_argument("--tasks", default="", help="comma list of task ids (default: all)")
    ap.add_argument("--models", default="sonnet", help="comma list of claude model aliases/ids")
    ap.add_argument("--arms", default="both", choices=["a", "b", "both"])
    ap.add_argument("--max-budget-usd", type=float, default=0.75, help="hard per-arm $ cap")
    ap.add_argument("--repeat", type=int, default=1, help="trials per (task,arm,model)")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--dry-run", action="store_true", help="print the matrix; no LLM, no spend")
    ap.add_argument("--selftest", action="store_true", help="no-LLM red->green check for all tasks")
    ap.add_argument("--list", action="store_true", help="list discovered tasks and exit")
    ap.add_argument("--out", default=os.path.join(HERE, "results.json"))
    args = ap.parse_args()

    only = {s for s in args.tasks.split(",") if s} or None
    tasks = load_tasks(only)
    if not tasks:
        print("no tasks found", file=sys.stderr)
        return 2

    if args.list:
        for t in tasks:
            print(f"  {t['difficulty']:<7} {t['id']:<26} {t.get('summary','')[:70]}")
        return 0
    if args.selftest:
        return selftest(tasks)

    models = [m for m in args.models.split(",") if m]
    arm_keys = ["a", "b"] if args.arms == "both" else [args.arms]

    if not args.dry_run and not shutil.which("claude"):
        print("`claude` CLI not found on PATH — cannot run the agent arms.", file=sys.stderr)
        return 2

    n_runs = len(tasks) * len(models) * len(arm_keys) * args.repeat
    print(f"{'DRY RUN: ' if args.dry_run else ''}matrix = {len(tasks)} task(s) × "
          f"{len(models)} model(s) × {len(arm_keys)} arm(s) × {args.repeat} = {n_runs} run(s)")

    results = []
    for model in models:
        for task in tasks:
            for _ in range(args.repeat):
                for arm_key in arm_keys:
                    results.append(run_one(task, arm_key, model, args.max_budget_usd,
                                           args.keep, args.dry_run))

    if args.dry_run:
        print("\nDRY RUN — no agent invoked, no quota spent.")
        return 0

    # Per-(task,model) tables (last trial of each arm).
    for model in models:
        for task in tasks:
            tm = [r for r in results if r["task"] == task["id"] and r["model"] == model]
            a = next((r for r in reversed(tm) if r["arm"] == "a"), None)
            b = next((r for r in reversed(tm) if r["arm"] == "b"), None)
            print_task_table(task["id"], task.get("difficulty"), model, a, b)

    scoreboard(results, models, arm_keys)

    with open(args.out, "w") as fh:
        json.dump({"models": models, "max_budget_usd": args.max_budget_usd,
                   "arms": arm_keys, "repeat": args.repeat, "results": results}, fh, indent=2)
    print(f"\nWrote {args.out}")
    print("\nNote: correctness first — a cheaper arm that didn't solve the task is not a win. "
          "Single trials are noisy; use --repeat. On easy tasks grep is expected to tie.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
