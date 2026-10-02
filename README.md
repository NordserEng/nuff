# nuff

Nordser's Python linter: [ruff](https://github.com/astral-sh/ruff)'s linter plus Nordser's house rules
(`NUF`), released as pinned wheels. It reads the same `[tool.ruff]` configuration, and the house rules
take theirs under `[tool.ruff.lint.nuff]`.

## Using it

A project pins each release wheel by URL, and its lockfile records each wheel's sha256. Fonno's
`backend/pyproject.toml` is the reference. Wheels exist for macOS arm64 and Linux x86_64/aarch64.

## Committing

A local gate keeps `main` releasable; GitHub runs nothing. Install it once per checkout with
`crates/nuff/gate install`. Before every commit, stage the complete change and run `crates/nuff/gate`. It
checks formatting, clippy, and the tests of `ruff_linter`, `ruff_workspace` and the `ruff` CLI in about
10–30 s against the incremental build. The hook refuses an unverified staged tree and any commit on
`main`, and stamps a verified commit `CI-Verified: nuff-gate`.

## Releasing

A release builds and publishes from an Apple Silicon Mac.

1. Set the same new version in `crates/nuff/Cargo.toml` and `crates/nuff/pyproject.toml`, and commit.
2. Run `crates/nuff/release` to build the wheels for macOS arm64 and Linux x86_64/aarch64 and check that
   each one reports the new version. The Linux wheels cross-compile through zig and run in Docker.
3. Run `crates/nuff/release --publish` to push the commit and the tag `nuff-<version>`, and to publish
   the wheels with their sha256.
4. In a consumer, point each wheel URL at the new release and run `uv lock`, which records the new hashes.

A published release is never rebuilt: consumers verify the bytes, so a fix is the next version.

## License

MIT, as ruff is. The code outside the house rules is Astral's; see `LICENSE`.
