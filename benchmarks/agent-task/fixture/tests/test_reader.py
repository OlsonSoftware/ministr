"""Reader / Table tests — all pass on the shipped fixture."""

import unittest

from minicsv import read_csv
from minicsv.errors import SchemaError

CSV = "city,temp\noslo,1.5\noslo,3.5\nlima,22.0\n"


class ReaderTest(unittest.TestCase):
    def test_header_and_len(self):
        t = read_csv(CSV)
        self.assertEqual(t.header, ["city", "temp"])
        self.assertEqual(len(t), 3)

    def test_numeric_coercion(self):
        t = read_csv(CSV)
        self.assertEqual(t.numeric_column("temp"), [1.5, 3.5, 22.0])

    def test_string_column_preserved(self):
        t = read_csv(CSV)
        self.assertEqual(t.column("city"), ["oslo", "oslo", "lima"])

    def test_missing_column_raises(self):
        t = read_csv(CSV)
        with self.assertRaises(SchemaError):
            t.column("humidity")


if __name__ == "__main__":
    unittest.main()
