# nuff

Nordser's ruff: the upstream `ruff` CLI, built as its own binary so Nordser's house rules can ship
without waiting on upstream. It reads the same `[tool.ruff]` configuration.

## Releasing

1. Set the same new version in `crates/nuff/Cargo.toml` and `crates/nuff/pyproject.toml`.
2. Merge to `main`, then push the tag `nuff-<version>` on that commit.
3. `nuff release` builds wheels for macOS arm64 and Linux x86_64/aarch64 and publishes them with their
   sha256 on the release.
4. In a consumer, point each wheel URL at the new release and run `uv lock`, which records the new hashes.

A published release is never rebuilt: consumers verify the bytes, so a fix is the next version.

## Trying a rule on a real project

Build with `cargo build --release -p nuff` and run `target/release/nuff check --no-cache` inside the
project. The lint cache keys on the version, so two local builds of one version share it and the
second reports the first one's results.
