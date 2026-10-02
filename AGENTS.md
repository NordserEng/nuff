# nuff — agent conventions

nuff is ruff's linter, trimmed to what the `nuff` binary and its tests need, plus Nordser's house rules.
The `README.md` covers using, committing and releasing.

- `crates/nuff` is the binary, its wheel config, the commit gate and the release script.
- The house rules live in `crates/ruff_linter/src/rules/nuff`: one file per rule, with passing and
  failing fixtures in `crates/ruff_linter/resources/test/fixtures/nuff` and insta snapshots beside the
  rules. A rule's options sit in `crates/ruff_linter/src/rules/nuff/settings.rs`, and
  `NuffOptions` in `crates/ruff_workspace/src/options.rs` reads them from `[tool.ruff.lint.nuff]`.
- `scripts/add_rule.py --linter nuff --prefix NUF --code <nnn> --name <Name> --category restriction`
  scaffolds a rule. House rules are `Restriction`, so preview mode never turns them on by default.
- Every other crate is upstream ruff. Change it only when a house rule needs it.

Tests come first: a new rule or a fix starts with a fixture case whose snapshot shows the wrong result.
To try a rule on a real project, build with `cargo build --release -p nuff` and run
`target/release/nuff check --no-cache` there. The lint cache keys on the version, so two local builds of
one version share it.

Before every commit: `git add -A && crates/nuff/gate`. Never `--no-verify`.
