# StreamFlow Tools

This directory contains utility scripts for StreamFlow development and testing.

## Scripts

### test.sh

Unified test runner with coverage support.

**Features:**
- Automatic database setup and teardown
- Test coverage tracking with cargo-llvm-cov
- Multiple output formats (terminal, HTML, lcov)
- Selective test execution (unit, integration, doc)
- Package-specific testing
- CI/CD integration support

**Usage:**
```bash
./tools/test.sh [OPTIONS]
```

**Common Commands:**
```bash
# Run all tests
./tools/test.sh

# Run with coverage report
./tools/test.sh --coverage-html

# Test specific crate
./tools/test.sh -p streamflow-api

# Unit tests only
./tools/test.sh --unit

# See all options
./tools/test.sh --help
```

**Coverage Options:**
- `--coverage` - Run with coverage (terminal summary)
- `--coverage-html` - Generate HTML report and open in browser
- `--coverage-ci` - Generate lcov format for CI/CD

**Test Selection:**
- `--unit` - Run only unit tests (--lib)
- `--integration` - Run only integration tests (--tests)
- `--doc` - Run only documentation tests
- `-p, --package CRATE` - Test specific crate

**Other Options:**
- `--verbose, -v` - Verbose output
- `--nocapture` - Don't capture test output
- `--skip-db-setup` - Skip database setup (assumes DB is ready)
- `--install-coverage` - Install cargo-llvm-cov

### setup-dev-db.sh

Sets up the development PostgreSQL database.

**Usage:**
```bash
./tools/setup-dev-db.sh
```

This script:
- Starts PostgreSQL container if not running
- Creates development database
- Runs migrations

## Requirements

### For Testing

- Rust toolchain (stable)
- Docker and docker-compose (for PostgreSQL)
- sqlx-cli (for migrations)
- cargo-llvm-cov (for coverage - install with `./tools/test.sh --install-coverage`)

### Installation

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Install coverage tool
./tools/test.sh --install-coverage
```

## Coverage Reports

Coverage reports are generated in the following locations:

- **HTML Report**: `target/llvm-cov/html/index.html`
- **lcov Report**: `coverage/lcov.info`

These paths are in `.gitignore` and will not be committed.

## CI/CD Integration

For continuous integration, use:

```bash
./tools/test.sh --coverage-ci --skip-db-setup
```

This generates an lcov report that can be uploaded to services like Codecov or Coveralls.

Example GitHub Actions workflow snippet:

```yaml
- name: Run tests with coverage
  run: ./tools/test.sh --coverage-ci --skip-db-setup
  env:
    DATABASE_URL: postgres://streamflow:streamflow_dev@localhost:5433/streamflow_test

- name: Upload coverage to Codecov
  uses: codecov/codecov-action@v3
  with:
    files: coverage/lcov.info
```

## Development Workflow

### Before Committing

```bash
# Run tests locally
./tools/test.sh

# Check coverage
./tools/test.sh --coverage
```

### When Adding New Code

```bash
# Test specific crate you're working on
./tools/test.sh -p streamflow-api

# Check coverage for that crate
./tools/test.sh --coverage-html -p streamflow-api
```

### Debugging Failed Tests

```bash
# Run with verbose output
./tools/test.sh --verbose

# Show test output
./tools/test.sh --nocapture

# Both together
./tools/test.sh --verbose --nocapture
```

## See Also

- [Testing Guide](../docs/testing.md) - Comprehensive testing documentation
- [Architecture](../docs/architecture.md) - System architecture
- [MVP Requirements](../docs/mvp-requirements.md) - Feature requirements
