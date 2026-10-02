# nuff

Nordser's ruff: the upstream `ruff` CLI, built as its own binary so Nordser's house rules can ship
without waiting on upstream. It reads the same `[tool.ruff]` configuration.

## Releasing

A release builds and publishes from an Apple Silicon Mac, so it costs no CI minutes. The fork runs no
GitHub Actions.

1. Set the same new version in `crates/nuff/Cargo.toml` and `crates/nuff/pyproject.toml`, and commit.
2. Run `crates/nuff/release` to build the wheels for macOS arm64 and Linux x86_64/aarch64 and check that
   each one reports the new version. The Linux wheels cross-compile through zig and run in Docker.
3. Run `crates/nuff/release --publish` to push the commit and the tag `nuff-<version>`, and to publish
   the wheels with their sha256.
4. In a consumer, point each wheel URL at the new release and run `uv lock`, which records the new hashes.

A published release is never rebuilt: consumers verify the bytes, so a fix is the next version.
