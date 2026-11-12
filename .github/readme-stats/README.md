# Profile stats automation

This Rust program replaces the marked GitHub stats section of the profile README.
The workflow runs daily at 05:23 UTC, on changes to the automation on `main`,
and through **Run workflow** in GitHub Actions.

## Build once, reuse the binary

The compiler is pinned to **Rust 1.96.0** in `rust-toolchain.toml`, and dependency
versions are recorded in `Cargo.lock`. The workflow restores the finished release
binary from GitHub Actions cache. Its key includes the Rust source and GraphQL
queries, manifest, lockfile, toolchain, workflow, runner architecture, and Ubuntu
version. It never falls back to a binary built from different source.

On an exact cache hit, the workflow runs the binary directly: there are no Cargo
commands, tests, or toolchain setup. On a miss, it installs the pinned toolchain,
checks formatting and Clippy, runs the tests, builds with `--release --locked`,
and saves the resulting binary. Changing the code invalidates the cache. Cache
eviction or unavailability also causes a rebuild, so this does not depend on
the cache being permanent. The runner is pinned to Ubuntu 24.04 for compatibility.

## Authentication

The workflow reuses the existing `GH_SECRET` repository secret to read GitHub
activity. Keep its existing repository read access if you want private activity
included in the aggregate. Repository names and commit details are never rendered.
Without that secret, the workflow falls back to `github.token`, whose access is
limited; cross-repository private activity may be missing.

The binary makes HTTPS requests directly to GitHub; daily runs need no GitHub CLI
or language runtime. Set `GH_TOKEN` or `GITHUB_TOKEN` locally, or use `gh auth login`
for the optional local credential lookup. Credentials are read at runtime.

README commits use the workflow's built-in token with `contents: write`,
independently of the token used to read stats. `WAKATIME_API_KEY` is no longer used
and can be deleted from the repository secrets.

## Metrics and scope

- Rolling 7-day and 30-day commit counts, active days, repository counts, and
  lines added/removed.
- Pull requests opened, reviews submitted, and issues opened in the past 30 days.
- Commit weekday and time-of-day distributions in `Europe/Berlin`, including DST.
- Languages by source bytes across all public, owned, non-fork repositories.

The contribution API identifies repositories with commit contributions during the
past 30 days. Authored commits are then paginated from each repository's default
branch, using their commit timestamps. Merge commits are excluded, and identical
commit hashes are counted once across repositories. Feature-branch work that has
not reached the default branch is outside this measure. Line counts include
generated files; they are diff totals, not a count of code currently maintained.
GitHub activity cannot determine coding time, editor/OS usage, or AI usage.

The API limits contribution discovery to 100 repositories. If that limit or the
100-language limit per repository is exceeded, the generator fails explicitly
instead of publishing an incomplete result. Commit histories and owned repository
lists are paginated. Temporary network/API failures are retried up to four times
with 2, 4, and 8 second backoffs and a 90 second timeout per request. Persistent
failures leave the README unchanged and fail the workflow normally.

## Run locally

From the repository root, preview without changing the README:

```sh
cd .github/readme-stats
cargo run --locked -- --user gabref --readme ../../README.md --dry-run
```

Omit `--dry-run` to update the README. Use `--timezone Europe/Berlin` to select
the reporting timezone.

From the Rust project directory:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Validate the workflow from the repository root with
`actionlint .github/workflows/main.yml`.
