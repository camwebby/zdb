# Releasing zdb

Releases use [cargo-dist](https://github.com/axodotdev/cargo-dist), pinned in
`dist-workspace.toml`. A version tag builds macOS (Apple Silicon and Intel) and
x86-64 Linux binaries, SHA-256 checksums, a shell installer, and a Homebrew formula.
OpenSSL is bundled into Linux release builds. SQLite is already bundled.

## One-time setup

The public tap is [camwebby/homebrew-tap](https://github.com/camwebby/homebrew-tap).
Create a fine-grained GitHub personal access token restricted to that repository
with **Contents: read and write**. Add it to zdb's
[Actions secrets](https://github.com/camwebby/zdb/settings/secrets/actions)
as `HOMEBREW_TAP_TOKEN`. Renew it before it expires.
The release workflow uses `GITHUB_TOKEN` for zdb's own release assets.

Homebrew and the latest-version shell installer are available after the first
successful stable release. No crates.io publication is required.

## Publish a version

1. Set the version in `Cargo.toml`, run `cargo check` to update `Cargo.lock`, and
   add release notes under the matching version in `CHANGELOG.md`.
2. Run `cargo test` and `cargo clippy --all-targets`. If distribution configuration
   changes, install cargo-dist 0.33.0 and run `dist generate` followed by
   `dist generate --check`.
3. Commit and push the release changes. Tag that commit with the package version:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

   Use the actual version for subsequent releases. Pushing the tag publishes the
   release and updates the tap automatically; pull requests only validate the plan.
4. Check the **Release** workflow, then verify both install paths on a clean machine:

   ```bash
   brew install camwebby/tap/zdb
   zdb --version
   ```

   ```bash
   curl --proto '=https' --tlsv1.2 -LsSf https://github.com/camwebby/zdb/releases/latest/download/zdb-installer.sh | sh
   ~/.local/bin/zdb --version
   ```

Prerelease tags such as `v0.2.0-rc.1` create GitHub prereleases and do not update
the stable Homebrew formula. Keep generated `release.yml` changes in
`dist-workspace.toml` and regenerate rather than editing the workflow directly.
