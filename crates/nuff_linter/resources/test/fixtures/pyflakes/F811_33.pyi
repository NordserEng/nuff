# Regression test
# Explicit re-exports at module scope should not be flagged as redefined
# by class-scoped attributes with the same name.
from x import y as y

class Foo:
    y = 42  # OK — class attribute, different scope from module-level re-export
