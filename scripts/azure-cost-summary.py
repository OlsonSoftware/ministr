#!/usr/bin/env python3
"""PHASE4 chunk 6 — summarise ACA indexer-job execution-seconds + cost.

Reads `az containerapp job execution list --output json` on stdin and
prints a per-day breakdown plus an estimated monthly cost at the rate
configured via COST_PER_SECOND (defaults to the 4 vCPU / 4 GiB rate).
"""
import datetime as dt
import json
import os
import sys


def main() -> int:
    data = json.load(sys.stdin)
    rate = float(os.environ.get("COST_PER_SECOND", "0.0001156"))
    buckets: dict[str, list[float]] = {}
    now = dt.datetime.now(dt.timezone.utc)
    for ex in data:
        props = ex.get("properties", {}) or {}
        start = props.get("startTime")
        end = props.get("endTime") or now.isoformat()
        if not start:
            continue
        s = dt.datetime.fromisoformat(start.replace("Z", "+00:00"))
        e = dt.datetime.fromisoformat(end.replace("Z", "+00:00"))
        if e < s:
            continue
        day = s.date().isoformat()
        secs = (e - s).total_seconds()
        bucket = buckets.setdefault(day, [0.0, 0.0])
        bucket[0] += secs
        bucket[1] += 1
    total_secs = 0.0
    print(f"  {'day':<12}  {'execs':>5}  {'seconds':>10}  {'est. $':>8}")
    for day in sorted(buckets):
        secs, n = buckets[day]
        total_secs += secs
        print(f"  {day:<12}  {int(n):>5}  {secs:>10.1f}  {secs*rate:>8.4f}")
    days = max(len(buckets), 1)
    avg_per_day = total_secs / days
    projected = avg_per_day * 30 * rate
    print()
    print(f"  total active-seconds (last {days} sampled days): {total_secs:.1f}")
    print(f"  avg seconds/day: {avg_per_day:.1f}")
    print(f"  projected $/mo at this rate: ${projected:.2f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
