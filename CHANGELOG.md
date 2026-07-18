# Changelog

All notable changes to Kruxia Flow are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Quickstart `catalog` init container failed with curl exit 23 on a truly
  fresh machine: the `curlimages/curl` default user (uid 100) cannot write to
  a newly created root-owned volume. It now runs as root; the keygen one-shot
  also chmods the generated dev keys to 644 so the non-root distroless server
  can always read them.
- README, docs, and landing-page budget snippets used `limit_usd`, which the
  schema has never accepted (`BudgetSettings` field is `limit`) — every
  copy-pasted first workflow failed validation. All launch assets corrected
  (the `budget_limit_usd` cost-API *response* field is unchanged and was
  already correct).

- **LLM model catalog refreshed to July 2026** (`config/llm_models.yaml`),
  verified against provider pricing pages. Added: Claude Fable 5, Opus 4.8/4.7,
  Sonnet 5 (introductory pricing through 2026-08-31 — bump to $3/$15 on
  2026-09-01), GPT-5.6/5.5/5.4 families, Gemini 3.5/3.1. Corrected: removed
  nonexistent `claude-haiku-4-6` and wrong Opus 4.1 ID; 4.6+ Claude models now
  show 1M context/128K output with no long-context premium; removed retired
  models (Claude 3.x, Gemini 2.0/1.5 — Gemini 2.0 shut down 2026-06-01).
  Examples, README, and landing updated off dead model IDs (including the
  never-valid `claude-haiku-4-20250415`).

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
- **Compose unification**: the root `docker-compose.yml` is now the published
  quickstart file — pull-only (the `build:` section moved to
  `docker-compose.override.yml`, which Compose auto-loads for plain
  `docker compose up` in a checkout; `./docker` passes explicit `-f` lists and
  is unaffected), self-initializing (keygen + LLM-catalog one-shot containers
  with named volumes replace the bind-mounted `docker-keys/` and `config/`
  prerequisite), secure by default (`KRUXIAFLOW_INSECURE_DEV=true` opts in;
  fixed local OAuth client defaults otherwise, guarded by the new default
  127.0.0.1 API port binding). Redis now sits behind the `cache` compose
  profile with `KRUXIAFLOW_CACHE_PROVIDER` defaulting to `noop` (`./docker`
  enables both automatically; standalone users opt in via `COMPOSE_PROFILES`).
  Postgres/redis host ports moved to the override file and bind loopback only;
  the `platform: linux/amd64` pins are gone (images are multi-arch);
  `docker-compose.develop.yml` was renamed to `docker-compose.override.yml`
  and `quickstart/docker-compose.yml` removed.

- **License changed from AGPL-3.0 to Apache-2.0** (2026-07-15). All current and
  future releases are Apache-2.0; releases prior to this change remain AGPL-3.0.
  Client SDKs are Apache-2.0 as well (previously MIT); forthcoming Rust crates
  will be dual-licensed MIT OR Apache-2.0. Copyright holder: Kruxia Corp.
  See `docs/licensing-faq.md`.
- README and positioning rewritten around budgeted workflows (hard cost limits
  enforced in the engine); comparison table re-cut, human-in-the-loop approval
  gates documented as a first-class feature.

### Added

- **Insecure dev mode** (`--insecure-dev` / `KRUXIAFLOW_INSECURE_DEV`): local
  development without the OAuth token dance. Unauthenticated requests execute
  as a synthetic dev principal; presented tokens are still validated normally.
  Startup refuses non-loopback binds without a second explicit override, warns
  loudly, and surfaces the flag in `/health` and `/api/v1/info`. Production
  behavior with the flag absent is unchanged.
- **Quickstart** (the root `docker-compose.yml`, fetched straight from the
  repo's raw GitHub URL — no separate published copy): one curl +
  `KRUXIAFLOW_INSECURE_DEV=true docker compose up -d` runs Kruxia Flow +
  PostgreSQL on 127.0.0.1, with generated local keys and the LLM pricing
  catalog seeded — first budgeted workflow with zero auth steps. README
  Getting Started rewritten around this flow.
- Release CI (`.github/workflows/release.yml`): tagged releases build binaries
  (x86_64-linux-gnu, aarch64-linux-gnu, aarch64-macos) with SHA-256 checksums,
  publish multi-arch Docker images (`kruxia/kruxiaflow`), and create a GitHub
  Release. Fully static musl builds are deferred (musl does not currently build).
- `scripts/sqlx-prepare.sh`: regenerates the sqlx offline cache for the whole
  workspace including test targets, against a throwaway postgres — the `.sqlx`
  cache now covers `--all-targets` as CI requires.
- `NOTICE` file and `CHANGELOG.md` (this file).
