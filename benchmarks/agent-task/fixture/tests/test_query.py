"""Query / aggregate tests — all pass on the shipped fixture.

These use bug-independent aggregators (mean/median/min/max) so the only
failure on the buggy fixture is the sample-stdev test in test_stats.py.
"""

import unittest

from minicsv import read_csv, Query

CSV = "city,temp\noslo,1.0\noslo,3.0\nlima,20.0\nlima,24.0\n"


class QueryTest(unittest.TestCase):
    def setUp(self):
        self.table = read_csv(CSV)

    def test_group_mean(self):
        out = Query(self.table).group("city").agg("temp", "mean")
        self.assertAlmostEqual(out["oslo"], 2.0)
        self.assertAlmostEqual(out["lima"], 22.0)

    def test_group_median(self):
        out = Query(self.table).group("city").agg("temp", "median")
        self.assertAlmostEqual(out["oslo"], 2.0)
        self.assertAlmostEqual(out["lima"], 22.0)

    def test_whole_column_max(self):
        out = Query(self.table).agg("temp", "max")
        self.assertAlmostEqual(out["*"], 24.0)

    def test_whole_column_count(self):
        out = Query(self.table).agg("temp", "count")
        self.assertEqual(out["*"], 4)


if __name__ == "__main__":
    unittest.main()
