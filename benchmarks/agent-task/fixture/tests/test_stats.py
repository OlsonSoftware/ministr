"""Statistics tests. One of these fails on the shipped (buggy) fixture.

Canonical dataset [2, 4, 4, 4, 5, 5, 7, 9]: mean 5, Σ(x-mean)² = 32.
  population variance = 32 / 8     = 4.0      → population stdev = 2.0
  sample     variance = 32 / (8-1) ≈ 4.5714   → sample     stdev ≈ 2.13809
"""

import unittest

from minicsv import stats

DATA = [2, 4, 4, 4, 5, 5, 7, 9]


class StatsTest(unittest.TestCase):
    def test_mean(self):
        self.assertAlmostEqual(stats.mean(DATA), 5.0)

    def test_population_variance(self):
        self.assertAlmostEqual(stats.population_variance(DATA), 4.0)

    def test_population_standard_deviation(self):
        self.assertAlmostEqual(stats.population_stdev(DATA), 2.0)

    def test_sample_variance(self):
        # Bessel's correction: divide by N-1, not N.
        self.assertAlmostEqual(stats.sample_variance(DATA), 32.0 / 7.0)

    def test_sample_standard_deviation(self):
        # The headline failing test on the buggy fixture.
        self.assertAlmostEqual(stats.stdev(DATA), 2.138089935299395)

    def test_median_odd(self):
        self.assertAlmostEqual(stats.median([3, 1, 2]), 2.0)

    def test_median_even(self):
        self.assertAlmostEqual(stats.median([1, 2, 3, 4]), 2.5)

    def test_quantile(self):
        self.assertAlmostEqual(stats.quantile([0, 10], 0.5), 5.0)


if __name__ == "__main__":
    unittest.main()
