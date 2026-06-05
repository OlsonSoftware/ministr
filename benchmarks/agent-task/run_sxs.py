#!/usr/bin/env python3
"""Side-by-side agent benchmark: a real coding agent solving the SAME task
WITH ministr vs WITHOUT it, with a deterministic pass/fail validator.

The runner is **headless Claude Code** (`claude -p --output-format json`) — a
real coding agent, already authenticated, that reports token usage. Two arms,
differing ONLY in the discovery tool available:

  arm A "ministr"  : Read, Edit, Write, Bash  +  the ministr MCP server
                     (the corpus is pre-indexed with `ministr index`), and
                     NO Grep/Glob — discovery goes through ministr.
  arm B "grep"     : Read, Edit, Write, Bash, Grep, Glob  and NO ministr.

Both run on a fresh copy of the fixture in a throwaway /tmp dir, must make the
test suite pass, and are measured on: validator pass/fail, input/output/cache
tokens, num_turns, total_cost_usd, wall-clock. A side-by-side table is printed
and written to a results JSON.

This calls a real LLM and spends real quota, so it is OPT-IN and never part of
any default gate. Cost is hard-capped per arm with `--max-budget-usd`.

Usage:
  python3 run_sxs.py --selftest          # deterministic, no LLM: broken fails / golden passes
  python3 run_sxs.py --dry-run           # print the exact commands, no LLM, no spend
  python3 run_sxs.py [--arms both] [--model sonnet] [--max-budget-usd 0.75] [--keep]

Reproducibility note: a real agent session is inherently non-deterministic. The
FIXTURE and validator are fully deterministic; the agent's path is not. Run
with `--repeat N` to average over N trials per arm.
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
FIXTURE = os.path.join(HERE, "fixture")
TASK_FILE = os.path.join(HERE, "task.md")
GOLDEN_STATS = os.path.join(HERE, "golden", "stats_fixed.py")

VALIDATE_CMD = ["python3", "-m", "unittest", "discover", "-s", "tests"]

ARMS = {
    # arm key -> (label, allowed tools, uses_ministr)
    "a": ("ministr", "Read Edit Write Bash mcp__ministr__*", True),
    "b": ("grep", "Read Edit Write Bash Grep Glob", False),
}


def sh(cmd, cwd=None, env=None, timeout=None):
    """Run a command, returning (exit_code, stdout, stderr)."""
    p = subprocess.run(
        cmd, cwd=cwd, env=env, timeout=timeout,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    return p.returncode, p.stdout, p.stderr


def make_workdir(tag):
    """Fresh throwaway copy of the fixture under /tmp."""
    work = tempfile.mkdtemp(prefix=f"sxs-{tag}-")
    dst = os.path.join(work, "fixture")
    shutil.copytree(FIXTURE, dst)
    return dst


def validate(workdir):
    """Run the hidden test suite. Returns (passed: bool, summary: str)."""
    code, out, err = sh(VALIDATE_CMD, cwd=workdir, timeout=120)
    tail = (err or out).strip().splitlines()
    summary = tail[-1] if tail else f"exit {code}"
    return code == 0, summary


def selftest():
    """Deterministic, no-LLM proof the fixture goes red->green on the fix."""
    broken = make_workdir("selftest-broken")
    ok_broken, sum_broken = validate(broken)
    fixed = make_workdir("selftest-fixed")
    shutil.copyfile(GOLDEN_STATS, os.path.join(fixed, "minicsv", "stats.py"))
    ok_fixed, sum_fixed = validate(fixed)
    shutil.rmtree(os.path.dirname(broken), ignore_errors=True)
    shutil.rmtree(os.path.dirname(fixed), ignore_errors=True)
    print(f"  broken fixture: passed={ok_broken}  ({sum_broken})")
    print(f"  golden  fixed : passed={ok_fixed}  ({sum_fixed})")
    good = (not ok_broken) and ok_fixed
    print("SELFTEST:", "OK — fixture is a valid red->green task" if good else "BROKEN")
    return 0 if good else 1


def build_claude_cmd(arm_key, workdir, prompt, model, budget, mcp_config_path):
    """Assemble the headless `claude -p` command for one arm."""
    label, allowed, uses_ministr = ARMS[arm_key]
    cmd = [
        "claude", "-p", prompt,
        "--output-format", "json",
        "--model", model,
        "--permission-mode", "bypassPermissions",
        "--no-session-persistence",
        "--setting-sources", "user",
        "--strict-mcp-config",           # ignore host MCP servers (isolation)
        "--add-dir", workdir,
        "--max-budget-usd", str(budget),
        "--allowedTools", allowed,
    ]
    if uses_ministr:
        cmd += ["--mcp-config", mcp_config_path]
    return cmd


def run_arm(arm_key, prompt, model, budget, keep, dry_run):
    """Set up, run one arm, validate, and return a result dict."""
    label, allowed, uses_ministr = ARMS[arm_key]
    workdir = make_workdir(label)
    result = {
        "arm": arm_key, "label": label, "uses_ministr": uses_ministr,
        "allowed_tools": allowed, "workdir": workdir,
    }

    mcp_config_path = None
    index_secs = None
    if uses_ministr:
        # Pre-index the fixture synchronously so survey works immediately.
        # The throwaway corpus is keyed by this /tmp path — it never touches
        # the user's real corpora.
        mcp_config = {"mcpServers": {"ministr": {
            "command": "ministr", "args": ["serve", "--corpus", workdir]}}}
        mcp_config_path = os.path.join(os.path.dirname(workdir), "ministr-mcp.json")
        with open(mcp_config_path, "w") as fh:
            json.dump(mcp_config, fh)
        # Index the fixture as the corpus. Run from cwd=workdir with
        # `--corpus .`: a `.ministr.toml` in the *current* dir takes precedence
        # over an absolute --corpus, so the only safe way to target the fixture
        # (and not whatever repo the harness was launched from) is to run inside
        # it. The fixture has no .ministr.toml, so `.` resolves to it.
        index_cmd = ["ministr", "index", "--corpus", "."]
        if dry_run:
            print(f"[{label}] INDEX (cwd={workdir}): {' '.join(index_cmd)}")
        else:
            t0 = time.time()
            code, out, err = sh(index_cmd, cwd=workdir, timeout=900)
            index_secs = round(time.time() - t0, 1)
            if code != 0:
                result["error"] = f"ministr index failed (exit {code}): {err[-400:]}"
                return result
    result["index_secs"] = index_secs

    cmd = build_claude_cmd(arm_key, workdir, prompt, model, budget, mcp_config_path)
    if dry_run:
        print(f"[{label}] cwd={workdir}")
        print(f"[{label}] RUN: {_quote(cmd)}")
        result["dry_run"] = True
        if not keep:
            shutil.rmtree(os.path.dirname(workdir), ignore_errors=True)
        return result

    t0 = time.time()
    code, out, err = sh(cmd, cwd=workdir, timeout=1800)
    result["wall_secs"] = round(time.time() - t0, 1)

    try:
        payload = json.loads(out)
    except json.JSONDecodeError:
        result["error"] = f"could not parse claude JSON (exit {code}): {out[-400:]} {err[-200:]}"
        return result

    usage = payload.get("usage", {}) or {}
    result.update({
        "is_error": payload.get("is_error"),
        "num_turns": payload.get("num_turns"),
        "total_cost_usd": payload.get("total_cost_usd"),
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "cache_read_tokens": usage.get("cache_read_input_tokens"),
        "cache_creation_tokens": usage.get("cache_creation_input_tokens"),
        "result_text": (payload.get("result") or "")[:280],
    })

    passed, summary = validate(workdir)
    result["validator_passed"] = passed
    result["validator_summary"] = summary

    if not keep:
        shutil.rmtree(os.path.dirname(workdir), ignore_errors=True)
        if mcp_config_path and os.path.exists(mcp_config_path):
            os.remove(mcp_config_path)
    return result


def _quote(cmd):
    out = []
    for c in cmd:
        out.append(f'"{c}"' if (" " in c or "\n" in c) else c)
    return " ".join(out)


def _tok(n):
    return "—" if n is None else f"{n:,}"


def print_table(results, model):
    print()
    print(f"=== Side-by-side agent benchmark (runner: claude -p, model: {model}) ===")
    cols = ["metric"] + [r["label"] for r in results]
    rows = [
        ("solved?", "validator_passed", lambda v: "—" if v is None else ("YES" if v else "NO")),
        ("turns", "num_turns", _tok),
        ("input tok", "input_tokens", _tok),
        ("output tok", "output_tokens", _tok),
        ("cache-read tok", "cache_read_tokens", _tok),
        ("cost USD", "total_cost_usd", lambda v: "—" if v is None else f"${v:.4f}"),
        ("wall s", "wall_secs", lambda v: "—" if v is None else f"{v:.0f}"),
        ("index s", "index_secs", lambda v: "—" if v is None else f"{v:.0f}"),
    ]
    # simple fixed-width print
    label_w = 16
    cell_w = max(12, *(len(r["label"]) for r in results))
    header = "metric".ljust(label_w) + "".join(c.rjust(cell_w) for c in cols[1:])
    print(header)
    print("-" * len(header))
    for name, key, fmt in rows:
        line = name.ljust(label_w) + "".join(fmt(r.get(key)).rjust(cell_w) for r in results)
        print(line)
    for r in results:
        if r.get("error"):
            print(f"\n[{r['label']}] ERROR: {r['error']}")


def main():
    ap = argparse.ArgumentParser(description="Side-by-side agent benchmark (ministr vs grep).")
    ap.add_argument("--arms", default="both", choices=["a", "b", "both"],
                    help="which arms to run (a=ministr, b=grep, both)")
    ap.add_argument("--model", default="sonnet", help="claude model alias or id")
    ap.add_argument("--max-budget-usd", type=float, default=0.75,
                    help="hard per-arm $ cap passed to claude --max-budget-usd")
    ap.add_argument("--repeat", type=int, default=1, help="trials per arm (averaged in JSON)")
    ap.add_argument("--keep", action="store_true", help="keep /tmp workdirs for inspection")
    ap.add_argument("--dry-run", action="store_true",
                    help="print the exact commands without calling claude (no spend)")
    ap.add_argument("--selftest", action="store_true",
                    help="deterministic, no-LLM check that the fixture is a valid red->green task")
    ap.add_argument("--out", default=os.path.join(HERE, "results.json"))
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    if not os.path.exists(TASK_FILE):
        print(f"task.md not found at {TASK_FILE}", file=sys.stderr)
        return 2
    prompt = open(TASK_FILE).read()

    if not args.dry_run and not shutil.which("claude"):
        print("`claude` CLI not found on PATH — cannot run the agent arms.", file=sys.stderr)
        return 2

    arm_keys = ["a", "b"] if args.arms == "both" else [args.arms]
    all_results = []
    for trial in range(args.repeat):
        for key in arm_keys:
            if args.repeat > 1 and not args.dry_run:
                print(f"[trial {trial + 1}/{args.repeat}] running arm {key} ({ARMS[key][0]})...")
            all_results.append(run_arm(key, prompt, args.model,
                                       args.max_budget_usd, args.keep, args.dry_run))

    if args.dry_run:
        print("\nDRY RUN — no agent was invoked, no quota spent.")
        return 0

    # For the table, show the last trial of each arm (full JSON has them all).
    latest = {}
    for r in all_results:
        latest[r["arm"]] = r
    print_table([latest[k] for k in arm_keys if k in latest], args.model)

    with open(args.out, "w") as fh:
        json.dump({"model": args.model, "max_budget_usd": args.max_budget_usd,
                   "results": all_results}, fh, indent=2)
    print(f"\nWrote {args.out}")

    # Honest verdict (does NOT assume ministr wins). Headline on total_cost_usd:
    # with prompt caching, `input_tokens` is only the *uncached* sliver (often a
    # handful of tokens), so it is NOT a meaningful headline — total cost
    # aggregates input + output + cache-read/creation into one comparable number.
    if args.arms == "both" and "a" in latest and "b" in latest:
        a, b = latest["a"], latest["b"]
        if a.get("validator_passed") and b.get("validator_passed"):
            ca, cb = a.get("total_cost_usd"), b.get("total_cost_usd")
            print("\nBoth solved it (correct + tests green).")
            if ca and cb:
                pct = round(100 * (1 - ca / cb))
                verb = "cheaper" if ca < cb else "more expensive"
                print(f"  cost:   ministr ${ca:.4f} vs grep ${cb:.4f}  → ministr {abs(pct)}% {verb}")
            for name, key in (("turns", "num_turns"), ("output tok", "output_tokens"),
                              ("cache-read", "cache_read_tokens")):
                va, vb = a.get(key), b.get(key)
                if va and vb:
                    pct = round(100 * (1 - va / vb))
                    rel = "fewer" if va < vb else "more"
                    print(f"  {name}: ministr {va:,} vs grep {vb:,}  → {abs(pct)}% {rel}")
        elif a.get("validator_passed") != b.get("validator_passed"):
            winner = "ministr" if a.get("validator_passed") else "grep"
            print(f"\nOnly the {winner} arm produced a CORRECT solution — correctness "
                  "outranks tokens; the result stands as measured.")
        else:
            print("\nNOTE: neither arm solved the task — see the table; result stands as measured.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
