from __future__ import annotations


# Test case
def f():
    x = 0
    list()[x:]


# Test case
def f():
    KeyTupleT = tuple[str, ...]

    keys_checked: set[KeyTupleT] = set()
    return keys_checked
