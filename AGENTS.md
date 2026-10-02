# nuff — agent conventions

nuff is Nordser's Python linter: pyflakes, `PLC0415`, flake8-async and the `NUF` house rules, behind one
`nuff check` command. The `README.md` covers using, committing and releasing.

- `crates/nuff` is the binary and its CLI, its wheel config, the commit gate and the release script.
- The house rules live in `crates/nuff_linter/src/rules/nuff`: one file per rule, with passing and
  failing fixtures in `crates/nuff_linter/resources/test/fixtures/nuff` and insta snapshots beside the
  rules. A rule's options sit in `crates/nuff_linter/src/rules/nuff/settings.rs`, and
  `NuffOptions` in `crates/nuff_workspace/src/options.rs` reads them from `[tool.nuff.lint.nuff]`.
- `scripts/add_rule.py --linter nuff --prefix NUF --code <nnn> --name <Name> --category restriction`
  scaffolds a rule. House rules are `Restriction`, so preview mode never turns them on by default.
- nuff carries only the rules a consumer selects. Another rule comes from ruff, which nuff was trimmed
  from: copy its module, fixtures and snapshots into `nuff_linter`, register its code in `codes.rs`, and
  call it from the checker.

Tests come first: a new rule or a fix starts with a fixture case whose snapshot shows the wrong result.
To try a rule on a real project, build with `cargo build --release -p nuff` and run
`target/release/nuff check --no-cache` there. The lint cache keys on the version, so two local builds of
one version share it.

Before every commit: `git add -A && crates/nuff/gate`. Never `--no-verify`.
