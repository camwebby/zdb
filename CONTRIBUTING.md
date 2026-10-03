# Contributing

Thanks for helping improve zdb!

## Getting started

```bash
git clone https://github.com/camwebby/zdb.git
cd zdb
cargo test
cargo clippy --all-targets
```

Integration tests run against live databases and are skipped unless `ZDB_TEST_PG` / `ZDB_TEST_MYSQL`
are set. See the Development section of the [README](README.md) for Docker commands.

## Pull requests

- Keep changes focused, and add tests where practical.
- Make sure `cargo test` and `cargo clippy --all-targets` pass.
- For larger changes, open an issue first to discuss the approach.
- Contributions are licensed under the project's [MIT license](LICENSE).

## Platforms

zdb is developed and tested on macOS. Linux is built in CI; Windows is untested. Reports and fixes welcome.
