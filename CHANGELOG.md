# Changelog

All notable changes to Kruxia Flow are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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
- `NOTICE` file and `CHANGELOG.md` (this file).
