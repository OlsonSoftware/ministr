You are working in a tiny Python module that converts integers to Roman
numerals. Its test suite is failing on the subtractive cases:

    to_roman(4)   returns "IIII"  but should be "IV"
    to_roman(9)   returns "VIIII" but should be "IX"
    to_roman(40)  returns "XXXX"  but should be "XL"   (and 90, 400, 900, …)

The converter only emits the additive symbols and never the subtractive forms.
Fix it so all the subtractive cases are correct.

Constraints:
- Do NOT edit any file under `tests/`.
- All tests must pass:  `python3 -m unittest discover -s tests`
- Make the smallest correct change.
