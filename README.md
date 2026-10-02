# nuff

Nordser's Python linter, released as pinned wheels. It runs pyflakes (`F`), `PLC0415`, flake8-async
(`ASYNC`) and Nordser's house rules (`NUF`), and nothing else.

## Using it

A project pins each release wheel by URL, and its lockfile records each wheel's sha256. Fonno's
`backend/pyproject.toml` is the reference. Wheels exist for macOS arm64 and Linux x86_64/aarch64.

nuff reads `[tool.nuff]` in `pyproject.toml`, or a `nuff.toml`. The house rules take their options under
`[tool.nuff.lint.nuff]`:

```toml
[tool.nuff.lint]
select = ["F", "PLC0415", "ASYNC", "NUF"]

[tool.nuff.lint.nuff]
blocking-functions = ["app.platform.email.send_email"]
```

`nuff check` is the only command. A `# noqa: F401` silences one line, and `# nuff: noqa: F401` a whole
file.

## Committing

A local gate keeps `main` releasable; GitHub runs nothing. Install it once per checkout with
`crates/nuff/gate install`. Before every commit, stage the complete change and run `crates/nuff/gate`. It
checks formatting, clippy, and the tests of `nuff_linter`, `nuff_workspace` and the `nuff` CLI in about
10–30 s against the incremental build. The hook refuses an unverified staged tree and any commit on
`main`, and stamps a verified commit `CI-Verified: nuff-gate`.

## Releasing

A release builds and publishes from an Apple Silicon Mac.

1. Set the same new version in the root `Cargo.toml` and `crates/nuff/pyproject.toml`, and commit.
2. Run `crates/nuff/release` to build the wheels for macOS arm64 and Linux x86_64/aarch64 and check that
   each one reports the new version. The Linux wheels cross-compile through zig and run in Docker.
3. Run `crates/nuff/release --publish` to push the commit and the tag `nuff-<version>`, and to publish
   the wheels with their sha256.
4. In a consumer, point each wheel URL at the new release and run `uv lock`, which records the new hashes.

A published release is never rebuilt: consumers verify the bytes, so a fix is the next version.

## License

MIT; see `LICENSE`.
