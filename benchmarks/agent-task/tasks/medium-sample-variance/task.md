You are working in a small Python library called `minicsv` (an in-memory CSV
analytics toolkit). Its test suite has a failing test:

    tests/test_stats.py :: StatsTest :: test_sample_standard_deviation

The sample standard deviation it returns is too small. It looks like the
sample variance is being computed as a *population* variance — dividing the
sum of squared deviations by N — instead of an unbiased *sample* variance,
which must apply Bessel's correction and divide by N − 1.

Find the function responsible and fix it so the sample standard deviation is
correct. There are several variance-related functions in the codebase; change
only the one that is actually wrong, and do not weaken the population
estimators or the streaming one.

Constraints:
- Do NOT edit any file under `tests/`.
- All tests must pass:  `python3 -m unittest discover -s tests`
- Make the smallest correct change.

When you are done, the full test suite should pass.
