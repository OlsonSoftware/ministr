"""Roman-numeral tests. The subtractive cases fail on the buggy fixture."""

import unittest

from roman import to_roman, from_roman


class ToRomanTest(unittest.TestCase):
    def test_additive(self):
        self.assertEqual(to_roman(3), "III")
        self.assertEqual(to_roman(2026), "MMXXVI")

    def test_subtractive_units(self):
        self.assertEqual(to_roman(4), "IV")
        self.assertEqual(to_roman(9), "IX")

    def test_subtractive_tens_hundreds(self):
        self.assertEqual(to_roman(40), "XL")
        self.assertEqual(to_roman(90), "XC")
        self.assertEqual(to_roman(400), "CD")
        self.assertEqual(to_roman(900), "CM")

    def test_canonical_year(self):
        self.assertEqual(to_roman(1994), "MCMXCIV")

    def test_roundtrip(self):
        for n in (4, 9, 49, 944, 1994, 3888):
            self.assertEqual(from_roman(to_roman(n)), n)

    def test_range(self):
        with self.assertRaises(ValueError):
            to_roman(0)
        with self.assertRaises(ValueError):
            to_roman(4000)


if __name__ == "__main__":
    unittest.main()
