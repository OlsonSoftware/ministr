#!/usr/bin/env python3
"""Grep-resistance validity gate for synthetic-wiredispatch.

The task is only valid if a grep-only strategy CANNOT localize the fix:
run the validator on the bugged fixture, tokenize everything an agent
could observe (the failing output + the task prompt), grep every token
across the fixture tree, and FAIL if the fix file appears in the union
of matched files. Repeatable, no LLM, no spend.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "fixture")
FIX_FILE = "ops/registry.list"

def sh(cmd, cwd=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=600)
    return r.returncode, r.stdout + r.stderr

def main():
    # 1. The observable symptom: failing validator output + the prompt.
    _, test_out = sh(["cargo", "test", "--quiet"], cwd=FIXTURE)
    prompt = open(os.path.join(HERE, "task.md")).read()
    observable = test_out + "\n" + prompt

    # 2. Tokenize: identifiers, hex literals, words >= 4 chars, dotted/
    #    pathy fragments. Drop bare numbers and rust/test boilerplate that
    #    matches everything ("assert", "thread", "panicked", file paths of
    #    the tests themselves are excluded from hits below, not here).
    tokens = set(re.findall(r"[A-Za-z_][A-Za-z0-9_]{3,}|0x[0-9A-Fa-f]+", observable))
    noise = {"left", "right", "thread", "panicked", "note", "test", "tests",
             "failed", "failures", "result", "passed", "ignored", "measured",
             "filtered", "finished", "assertion", "stdout", "RUST_BACKTRACE",
             "environment", "variable", "display", "backtrace", "error",
             "rerun", "pass", "cargo", "quiet", "conformance", "running",
             "unittests", "running", "doctests", "relaykit", "When", "believe",
             "instead", "written", "protocol", "public", "contract", "correct",
             "modify", "anything", "under", "crate", "suite", "wire",
             "libraries", "client", "released", "mirror", "implements",
             "operation", "codes", "used", "confirm", "passes"}
    tokens = {t for t in tokens if t not in noise}

    # 3. Grep each token; collect matched files (excluding tests/ + docs of
    #    the suite itself is NOT done — every fixture file counts).
    hits = {}
    for tok in sorted(tokens):
        code, out = sh(["grep", "-rIl", "--exclude-dir=target", tok, "."], cwd=FIXTURE)
        files = {f.lstrip("./") for f in out.splitlines() if f.strip()}
        if files:
            hits[tok] = files

    # A token only LOCALIZES if its match-set is small: grep's value is
    # narrowing, and a token matching half the tree (English words like
    # "this") gives an agent nothing. Threshold: 5 files.
    localizing = {t: fs for t, fs in hits.items() if len(fs) <= 5}
    union = set().union(*localizing.values()) if localizing else set()
    fix_hits = [t for t, fs in localizing.items() if FIX_FILE in fs]

    print(f"tokens probed: {len(tokens)}; files in union: {len(union)}")
    if fix_hits:
        print(f"PROBE FAILED — fix file {FIX_FILE} reachable via tokens: {fix_hits}")
        return 1
    print(f"PROBE PASSED — {FIX_FILE} unreachable from any observable token")
    print("union (what grep CAN reach):", ", ".join(sorted(union)) or "(nothing)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
