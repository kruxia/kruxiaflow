# StreamFlow Testing Guide

This guide covers testing practices, test coverage tracking, and testing workflows for the StreamFlow project.

## Table of Contents

- [Running Tests](#running-tests)
- [Test Coverage](#test-coverage)
- [Test Organization](#test-organization)
- [Coverage Goals](#coverage-goals)
- [CI/CD Integration](#cicd-integration)

## Running Tests

StreamFlow uses a unified test script at `tools/test.sh` for all testing needs.

### Basic Usage

```bash
# Run all tests
./tools/test.sh

# Run with verbose output
./tools/test.sh --verbose

# Run tests for a specific crate
./tools/test.sh -p streamflow-api

# Show all options
./tools/test.sh --help
```

### Test Types

```bash
# Unit tests only (--lib)
./tools/test.sh --unit

# Integration tests only (--tests)
./tools/test.sh --integration

# Documentation tests only (--doc)
./tools/test.sh --doc
```

### Debugging Tests

```bash
# Show test output (don't capture)
./tools/test.sh --nocapture

# Verbose + nocapture
./tools/test.sh --verbose --nocapture
```

### Skipping Database Setup

```bash
# Skip DB setup if database is already configured
./tools/test.sh --skip-db-setup
```

## Test Coverage

StreamFlow uses `cargo-llvm-cov` for accurate, cross-platform test coverage tracking.

### Installing Coverage Tools

```bash
# Install cargo-llvm-cov
./tools/test.sh --install-coverage

# Or manually
cargo install cargo-llvm-cov
```

### Running Tests with Coverage

```bash
# Run tests with coverage summary in terminal
./tools/test.sh --coverage

# Generate HTML coverage report and open in browser
./tools/test.sh --coverage-html

# Generate lcov format for CI/CD (e.g., Codecov)
./tools/test.sh --coverage-ci
```

### Coverage Reports

After running with `--coverage-html`, the HTML report will be at:
```
target/llvm-cov/html/index.html
```

After running with `--coverage-ci`, the lcov report will be at:
```
coverage/lcov.info
```

### Coverage by Package

```bash
# Coverage for a specific crate
./tools/test.sh --coverage -p streamflow-api

# HTML report for API crate only
./tools/test.sh --coverage-html -p streamflow-api
```

## Test Organization

### Directory Structure

```
streamflow/
├── core/
│   ├── src/
│   │   └── *.rs              # Source files with inline unit tests
│   └── tests/
│       └── *_test.rs          # Integration tests
├── api/
│   ├── src/
│   │   └── *.rs              # Source files with inline unit tests
│   └── tests/
│       ├── health_integration_tests.rs
│       └── error_handling_test.rs
├── activity/
│   └── ...
└── tools/
    └── test.sh                # Unified test runner
```

### Test Naming Conventions

**Unit Tests** (inline with source):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // Test implementation
    }

    #[tokio::test]
    async fn test_async_function() {
        // Async test
    }
}
```

**Integration Tests** (in tests/ directory):
```rust
// tests/feature_integration_test.rs

#[tokio::test]
#[serial]  // Use serial_test for database tests
async fn test_feature_end_to_end() {
    // Integration test
}
```

### Test Markers

- `#[test]` - Standard synchronous test
- `#[tokio::test]` - Async test (requires tokio)
- `#[serial]` - Sequential execution (for database tests)
- `#[ignore]` - Skip by default (run with --ignored)

## Coverage Goals

StreamFlow aims for high test coverage to ensure production reliability:

### Target Coverage by Component

| Component | Target | Current | Notes |
|-----------|--------|---------|-------|
| **Core (Orchestrator)** | 90%+ | TBD | Critical path - workflow execution |
| **Core (Queue)** | 90%+ | TBD | Critical path - task distribution |
| **Core (Event Source)** | 85%+ | TBD | Core infrastructure |
| **API (Handlers)** | 85%+ | ~95% | User-facing endpoints |
| **API (Error Handling)** | 95%+ | 100% | Error paths must be tested |
| **Activity (Worker)** | 85%+ | TBD | Activity execution |
| **Dashboard** | 70%+ | TBD | UI/frontend code |
| **Overall** | 80%+ | TBD | Project-wide target |

### Exclusions

The following are excluded from coverage requirements:
- Generated code (build.rs)
- Test code itself (tests/)
- Example code (examples/)
- Dashboard frontend (TypeScript/React)

### Reviewing Coverage

```bash
# Generate HTML report for detailed analysis
./tools/test.sh --coverage-html

# Check specific module coverage
# (View in HTML report)
```

Look for:
- ✅ **Green lines**: Executed by tests
- ❌ **Red lines**: Not covered by tests
- ⚠️ **Yellow lines**: Partially covered (branches)

### Improving Coverage

1. **Identify gaps**: Run `--coverage-html` and review uncovered lines
2. **Write tests**: Add unit or integration tests for uncovered code
3. **Test error paths**: Ensure error handling is tested
4. **Test edge cases**: Boundary conditions, empty inputs, etc.
5. **Verify**: Re-run coverage to confirm improvement

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Tests and Coverage

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    
    services:
      postgres:
        image: postgres:15
        env:
          POSTGRES_USER: streamflow
          POSTGRES_PASSWORD: streamflow_dev
          POSTGRES_DB: streamflow_test
        ports:
          - 5433:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5

    steps:
      - uses: actions/checkout@v3
      
      - uses: dtolnay/rust-toolchain@stable
      
      - name: Cache dependencies
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo
            target/
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Install sqlx-cli
        run: cargo install sqlx-cli --no-default-features --features postgres
      
      - name: Install cargo-llvm-cov
        run: cargo install cargo-llvm-cov
      
      - name: Run migrations
        run: sqlx migrate run
        env:
          DATABASE_URL: postgres://streamflow:streamflow_dev@localhost:5433/streamflow_test
      
      - name: Run tests with coverage
        run: ./tools/test.sh --coverage-ci --skip-db-setup
        env:
          DATABASE_URL: postgres://streamflow:streamflow_dev@localhost:5433/streamflow_test
      
      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v3
        with:
          files: coverage/lcov.info
          fail_ci_if_error: true
```

### Local Pre-commit Workflow

```bash
# Before committing
./tools/test.sh --coverage

# Ensure no regressions
# - All tests pass
# - Coverage doesn't drop significantly
```

## Best Practices

### Writing Testable Code

1. **Small functions**: Easier to test in isolation
2. **Dependency injection**: Pass dependencies as parameters
3. **Avoid global state**: Makes tests independent
4. **Use traits**: Enable mocking and test doubles
5. **Error handling**: Return Result<T, E> for testability

### Test Quality

1. **Arrange-Act-Assert**: Clear test structure
2. **One assertion per test**: When possible
3. **Descriptive names**: `test_workflow_submission_with_invalid_json_returns_422`
4. **Test error paths**: Not just happy paths
5. **Avoid flaky tests**: Use `serial_test` for shared resources

### Database Tests

```rust
use serial_test::serial;

#[tokio::test]
#[serial]  // Database tests must not run in parallel
async fn test_database_operation() {
    let pool = setup_test_pool().await;
    
    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Migrations failed");
    
    // Test implementation
}
```

### Async Tests

```rust
#[tokio::test]
async fn test_async_function() {
    let result = async_function().await;
    assert_eq!(result, expected);
}
```

## Troubleshooting

### Coverage Tool Not Found

```bash
# Install it
./tools/test.sh --install-coverage

# Or manually
cargo install cargo-llvm-cov
```

### Tests Fail Only with Coverage

This is rare but can happen. Try:
```bash
# Clean and rebuild
cargo clean
./tools/test.sh --coverage
```

### Database Connection Issues

Ensure PostgreSQL is running:
```bash
# Start database
docker-compose up -d postgres

# Run tests
./tools/test.sh
```

The test script handles DATABASE_URL automatically.

### Slow Tests

```bash
# Run specific crate only
./tools/test.sh -p streamflow-api

# Run unit tests only (faster)
./tools/test.sh --unit

# Skip database setup if DB is already ready
./tools/test.sh --skip-db-setup
```

## Resources

- [cargo-llvm-cov documentation](https://github.com/taiki-e/cargo-llvm-cov)
- [Rust testing guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [tokio testing](https://tokio.rs/tokio/topics/testing)
- [serial_test crate](https://docs.rs/serial_test/)
