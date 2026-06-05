#!/usr/bin/env python3
"""Record ONE side-by-side run (ministr vs grep) as a replayable timeline.

Runs the same task through both arms with `claude -p --output-format stream-json`,
stamps the arrival time of every streamed event, and writes a compact
`race-data.json` the website animates as a real-time "race". The timeline is a
REAL recording (tool-call sequence + cumulative tokens over wall-clock), not a
synthesized animation.

Reuses the index-once realrepo machinery from run_sxs (clone+install+index the
base once; both arms reuse it).

Usage:
  python3 record_race.py --task realrepo-sympy-tribonacci --model sonnet
  python3 record_race.py --task realrepo-click --model haiku --max-budget-usd 0.4
"""

import argparse
import json
import os
import subprocess
import time

import run_sxs as R

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_OUT = os.path.abspath(os.path.join(HERE, "..", "..", "web", "components", "landing", "race-data.json"))

# Tools we surface as race "moves", mapped to a short display label.
def tool_label(name):
    if name.startswith("mcp__ministr__"):
        return name.split("__")[-1]          # ministr_survey → survey
    return name                               # Grep, Glob, Read, Edit, Bash, …


def stream_arm(cmd, cwd, env, timeout):
    """Run claude (stream-json) and return (events, final). events = list of
    {t, kind, name, out} stamped by arrival time; final = the result envelope."""
    t0 = time.monotonic()
    events = []
    final = {}
    cum_out = 0
    proc = subprocess.Popen(cmd, cwd=cwd, env=env, text=True, bufsize=1,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                e = json.loads(line)
            except json.JSONDecodeError:
                continue
            t = round(time.monotonic() - t0, 2)
            etype = e.get("type")
            msg = e.get("message") if isinstance(e.get("message"), dict) else {}
            usage = msg.get("usage") or e.get("usage") or {}
            if isinstance(usage, dict) and usage.get("output_tokens"):
                cum_out += usage["output_tokens"]
            if etype == "assistant":
                for c in (msg.get("content") or []):
                    if isinstance(c, dict) and c.get("type") == "tool_use":
                        inp = c.get("input") or {}
                        detail = (inp.get("command") or inp.get("query")
                                  or inp.get("file_path") or inp.get("pattern") or "")
                        events.append({"t": t, "kind": "tool",
                                       "name": tool_label(c.get("name", "?")),
                                       "detail": str(detail)[:80],
                                       "out": cum_out})
            elif etype == "result":
                u = e.get("usage") or {}
                final = {
                    "is_error": e.get("is_error"),
                    "num_turns": e.get("num_turns"),
                    "total_cost_usd": e.get("total_cost_usd"),
                    "output_tokens": u.get("output_tokens"),
                    "cache_read_tokens": u.get("cache_read_input_tokens"),
                    "wall": round(time.monotonic() - t0, 1),
                }
                events.append({"t": t, "kind": "done", "name": "done", "out": cum_out})
    finally:
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
    return events, final


def record_arm(task, base, arm_key, model, budget):
    label, allowed, uses_ministr = R.ARMS[arm_key]
    repo = base["repo"]
    R.reset_realrepo_base(task, repo)
    if uses_ministr:
        # Deployment-faithful steering: what `ministr init` puts in the repo.
        # (The reset's `git clean` removes it before each arm, so arm B never
        # sees it.)
        open(os.path.join(repo, "CLAUDE.md"), "w").write(R.MINISTR_CLAUDE_MD)
    prompt = open(os.path.join(task["_dir"], "task.md")).read()
    env = dict(os.environ)
    env["PATH"] = os.path.join(R.venv_of(repo), "bin") + os.pathsep + env.get("PATH", "")
    cmd = ["claude", "-p", prompt, "--output-format", "stream-json", "--verbose",
           "--model", model, "--permission-mode", "bypassPermissions",
           "--no-session-persistence", "--setting-sources", "user",
           "--strict-mcp-config", "--add-dir", repo,
           "--max-budget-usd", str(budget), "--allowedTools", allowed]
    if uses_ministr:
        mcp = {"mcpServers": {"ministr": {"command": "ministr", "args": ["serve", "--corpus", repo]}}}
        path = os.path.join(base["work"], "ministr-mcp.json")
        json.dump(mcp, open(path, "w"))
        # Steering itself comes from the CLAUDE.md written above (deployment-
        # faithful); here we only mirror the hook-level shell-grep block.
        cmd += ["--mcp-config", path,
                "--disallowedTools", R.MINISTR_ARM_DISALLOWED]
    events, final = stream_arm(cmd, repo, env, timeout=2400)
    passed, _ = R.validate(task, repo)
    return {
        "label": label, "uses_ministr": uses_ministr,
        "events": events,
        "tool_calls": sum(1 for e in events if e["kind"] == "tool"),
        "solved": passed,
        **final,
    }


def main():
    ap = argparse.ArgumentParser(description="Record a side-by-side run as race-data.json")
    ap.add_argument("--task", required=True)
    ap.add_argument("--model", default="sonnet")
    ap.add_argument("--max-budget-usd", type=float, default=0.75)
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()

    task = next((t for t in R.load_tasks({args.task}) if t["id"] == args.task), None)
    if task is None:
        raise SystemExit(f"task {args.task} not found")
    if task.get("kind") != "realrepo":
        raise SystemExit("record_race currently supports realrepo tasks only")

    print(f"preparing base for {task['id']} (clone+install+index once)...")
    base = R.prepare_realrepo_base(task, need_index=True)
    if base.get("error"):
        raise SystemExit(f"base setup failed: {base['error']}")
    print(f"indexed once in {base.get('index_secs')}s; recording both arms...")
    arms = {}
    for arm_key in ("a", "b"):
        print(f"  recording arm {arm_key} ({R.ARMS[arm_key][0]})...")
        arms[R.ARMS[arm_key][0]] = record_arm(task, base, arm_key, args.model, args.max_budget_usd)
    import shutil
    shutil.rmtree(base["work"], ignore_errors=True)

    out = {
        "task": task["id"],
        "repo": task.get("display") or os.path.basename(task.get("repo", task["id"])),
        "model": args.model,
        "index_secs": base.get("index_secs"),
        "arms": arms,
    }
    json.dump(out, open(args.out, "w"), indent=2)
    print(f"\nWrote {args.out}")
    for name, a in arms.items():
        print(f"  {name:<8} solved={a.get('solved')} tools={a.get('tool_calls')} "
              f"turns={a.get('num_turns')} out_tok={a.get('output_tokens')} "
              f"cost=${a.get('total_cost_usd')} wall={a.get('wall')}s")


if __name__ == "__main__":
    raise SystemExit(main())
