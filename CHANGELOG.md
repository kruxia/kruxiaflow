# Changelog

All notable changes to Kruxia Flow are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Docker builds broke when unpinned `cargo install sqlx-cli` began resolving
  to 0.9.0, which requires a newer rustc than the pinned base image. sqlx-cli
  is now pinned to 0.8.6 (matching the workspace's sqlx) and the base images
  bumped from `rust:1.90-bookworm` to `rust:1.97-bookworm`.

### Changed

- Release binaries now build with `--features redis-cache`, matching the
  Docker image's feature set.
- CI and releases unified into one pipeline (`main-ci.yml`): tag pushes run
  checks → binaries + versioned multi-arch Docker → GitHub Release, so nothing
  publishes unless clippy/fmt/tests pass; the redundant branch-push run for
  release commits is skipped (same SHA runs once, on the tag). Releases are
  cut with `cargo release <version>` (config under
  `[workspace.metadata.release]`); a preflight job fails fast if a tag doesn't
  match the workspace version.
- CI caching: prebuilt tool binaries (cargo-llvm-cov, sqlx-cli, cross) instead
  of source installs, cache-on-failure for cargo caches, Docker layer caching.

- **License changed from AGPL-3.0 to Apache-2.0** (2026-07-15). All current and
  future releases are Apache-2.0; releases prior to this change remain AGPL-3.0.
  Client SDKs are Apache-2.0 as well (previously MIT); forthcoming Rust crates
  will be dual-licensed MIT OR Apache-2.0. Copyright holder: Kruxia Corp.
  See `docs/licensing-faq.md`.
- README and positioning rewritten around budgeted workflows (hard cost limits
  enforced in the engine); comparison table re-cut, human-in-the-loop approval
  gates documented as a first-class feature.

### Added

- Release CI (`.github/workflows/release.yml`): tagged releases build binaries
  (x86_64-linux-gnu, aarch64-linux-gnu, aarch64-macos) with SHA-256 checksums,
  publish multi-arch Docker images (`kruxia/kruxiaflow`), and create a GitHub
  Release. Fully static musl builds are deferred (musl does not currently build).
- `scripts/sqlx-prepare.sh`: regenerates the sqlx offline cache for the whole
  workspace including test targets, against a throwaway postgres — the `.sqlx`
  cache now covers `--all-targets` as CI requires.
- `NOTICE` file and `CHANGELOG.md` (this file).
