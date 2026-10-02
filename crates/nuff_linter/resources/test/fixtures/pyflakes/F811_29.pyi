"""Regression test"""

from foo import Bar as Bar

class Eggs:
    Bar: int  # OK

Bar = 1  # F811
