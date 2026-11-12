# Profile stats automation

This Rust program updates the marked GitHub stats section and generates the SVG
dashboard cards in `assets/stats/`. The profile header is a separate static SVG.
The workflow runs daily at 05:23 UTC, on changes to the automation on `main`,
and through **Run workflow** in GitHub Actions.

## Responsive cards

Every dashboard image fills its container. The language/rhythm and stack/shipping
groups each have a two-column SVG and a stacked SVG. GitHub's `<picture>` support
selects the stacked asset below a 1200-pixel viewport. The selected SVG's own
aspect ratio sets its height, so stacked cards fill the entire README width.
The overview and fork cards also have compact layouts with larger text on phones.
The layout requires no JavaScript or CSS injected into GitHub. Standalone card
files remain available for direct embedding.

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
- Languages by source bytes across public and private owned, non-fork repositories.
- Authored commits on all branches of recently pushed owned forks and in PRs
  opened by the account, including commits not yet merged upstream.
- Fork-specific commit/line totals, PR counts, and changed-line languages based
  on file types. The fork's entire codebase is not treated as authored work.
- Open, merged and closed-without-merge outcomes for PRs opened in the past 30 days.
- A 30-date commit pulse, current commit streak and longest streak in that window,
  using the configured timezone. Today can still be empty without breaking a streak
  that ended yesterday; these are not lifetime streaks.
- Framework/tooling repository counts for Tauri, Android, Linux, Neovim, React,
  Vue, Astro and terminal tools.

Framework signals come from repository topics, declared package dependencies,
Cargo targets, Android manifests/Gradle files and Neovim config paths on owned
non-fork repositories. Each repository counts once per tool and can match multiple
tools. Manifest inspection is batched by eight repositories and pinned to each
repository's default-branch commit. Empty repositories can still match topics.
These counts describe repository evidence; Linux, Neovim and terminal preferences
in the introduction are self-described and do not claim measured usage time.
The detection is a bounded heuristic, not a recursive scan of every monorepo package.

The contribution API identifies repositories with commit contributions during the
past 30 days. Private repositories are also discovered explicitly because the
contribution feed can mask private details. Recently pushed private repositories
owned by the account or accessible through collaborations/organizations are scanned
on their default branches. Owned forks are scanned across all branches. PRs are
paginated by update date, with only commits authored by the account in the past
30 days included. Older PRs can still contribute recent commits.

Merge commits are excluded. Identical commit hashes are counted once across
default branches, fork branches and PRs, including after a PR is merged. The fork
subtotal overlaps the overall total. Fork PR counts cover PRs opened during the
reporting period with a fork as source or destination. Deleted source forks can
leave PRs without fork metadata; their authored commits still enter the overall
total. Work on unrelated feature branches outside these sources is not scanned.

Fork contribution languages use additions plus deletions per changed file from
GitHub's commit API, classified by file extension/name. They describe changed lines,
not source bytes or coding time. The main language card uses GitHub's source byte
counts for owned, non-fork repositories. Repository/branch names, PR titles, commit
messages, and changed-file paths never enter the README or SVGs.

Line counts include generated files; they are diff totals, not a count of code
currently maintained. GitHub's restricted/private contribution count is displayed
separately because it includes contribution types beyond commits.
GitHub activity cannot determine coding time, editor/OS usage, or AI usage.

The API limits contribution discovery to 100 repositories. If that limit or the
100-language limit per repository is exceeded, the generator fails explicitly
instead of publishing an incomplete result. Commit histories and owned repository
lists are paginated. Temporary network/API failures are retried up to four times
with 2, 4, and 8 second backoffs and a 90 second timeout per request. Persistent
failures leave the README and cards unchanged and fail the workflow normally.

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

## Inspiration

The tool sections and streak presentation in
[DenverCoder1's profile](https://github.com/DenverCoder1/DenverCoder1), and the
PR follow-up/calendar ideas in [lowlighter/metrics](https://github.com/lowlighter/metrics),
informed the PR outcome and commit consistency additions. The implementation and
SVGs are generated by this repository's Rust binary using the existing GitHub token.
