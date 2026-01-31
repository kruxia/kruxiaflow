# Contributing to Kruxia Flow

Thank you for your interest in contributing to Kruxia Flow! This document provides guidelines and information for contributors.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [How to Contribute](#how-to-contribute)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Documentation](#documentation)
- [Community](#community)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to [conduct@kruxia.com](mailto:conduct@kruxia.com).

## Getting Started

### Prerequisites

- **Docker and Docker Compose** - Required for running the development environment
- **Rust 1.90+** - Required for local development without Docker
- **PostgreSQL 17+** - The only required runtime dependency

### Quick Setup

```bash
# Clone the repository
git clone https://github.com/kruxia/kruxiaflow.git
cd kruxiaflow

# Start the development environment
./docker up --develop -d

# View logs
./docker logs -f

# Verify everything is working
curl http://localhost:8080/health
```

## Development Setup

### Using Docker (Recommended)

The easiest way to develop is using the included Docker Compose configuration:

```bash
# Start all services with hot reload
./docker up --develop

# Run in background
./docker up --develop -d

# View logs
./docker logs -f

# Stop services
./docker down
```

### Local Development (Without Docker)

For local development without Docker:

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install sqlx-cli for database management
cargo install sqlx-cli --no-default-features --features postgres

# Start PostgreSQL (required)
docker run -d --name pg -e POSTGRES_PASSWORD=dev -p 5432:5432 postgres:17

# Set up the database
export DATABASE_URL='postgres://postgres:dev@localhost:5432/kruxiaflow'
sqlx database create
sqlx migrate run

# Build the project
cargo build

# Run the server
cargo run --bin kruxiaflow -- serve
```

### Environment Variables

Copy the example environment file and configure as needed:

```bash
cp .env.example .env
```

Key environment variables:

| Variable           | Description                    | Default                                           |
|--------------------|--------------------------------|---------------------------------------------------|
| `DATABASE_URL`     | PostgreSQL connection string   | `postgres://postgres:dev@localhost:5432/kruxiaflow` |
| `KRUXIAFLOW_HOST`  | API server host                | `0.0.0.0`                                         |
| `KRUXIAFLOW_PORT`  | API server port                | `8080`                                            |
| `RUST_LOG`         | Log level                      | `info`                                            |

## How to Contribute

### Reporting Bugs

Before submitting a bug report:

1. Check the [existing issues](https://github.com/kruxia/kruxiaflow/issues) to avoid duplicates
2. Ensure you're using the latest version
3. Collect relevant information (logs, environment, reproduction steps)

When submitting a bug report, include:

- A clear, descriptive title
- Steps to reproduce the issue
- Expected vs actual behavior
- Environment details (OS, Docker version, Rust version)
- Relevant logs or error messages

### Suggesting Features

Feature requests are welcome! Please:

1. Check [existing issues](https://github.com/kruxia/kruxiaflow/issues) and [discussions](https://github.com/kruxia/kruxiaflow/discussions) first
2. Provide a clear use case for the feature
3. Explain how it fits with Kruxia Flow's goals (AI-native workflow orchestration with cost control)

### Contributing Code

1. **Find or create an issue** - Discuss your proposed changes before starting work
2. **Fork the repository** - Create your own fork to work in
3. **Create a branch** - Use a descriptive branch name (e.g., `feature/add-redis-backend`, `fix/memory-leak-orchestrator`)
4. **Make your changes** - Follow our coding standards
5. **Test your changes** - Ensure all tests pass
6. **Submit a pull request** - Reference the related issue

## Pull Request Process

### Before Submitting

- [ ] Code compiles without warnings (`cargo build`)
- [ ] All tests pass (`./scripts/test.sh`)
- [ ] Code follows project style guidelines (`cargo fmt --check`)
- [ ] Code passes linting (`cargo clippy`)
- [ ] Documentation is updated if needed
- [ ] Commit messages are clear and descriptive

### PR Guidelines

1. **Keep PRs focused** - One feature or fix per PR
2. **Write descriptive titles** - Summarize the change clearly
3. **Fill out the PR template** - Provide context and testing notes
4. **Link related issues** - Use `Fixes #123` or `Relates to #456`
5. **Respond to feedback** - Be open to suggestions and iterate

### Review Process

1. All PRs require at least one approval from a maintainer
2. CI checks must pass (build, tests, linting)
3. Maintainers may request changes or ask questions
4. Once approved, a maintainer will merge the PR

## Coding Standards

### Rust Style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` for consistent formatting
- Address all `cargo clippy` warnings
- Prefer explicit error handling over `.unwrap()` in production code
- Use meaningful variable and function names

### Project-Specific Guidelines

- **Database access**: Use sqlx with compile-time query validation
- **Async code**: Use Tokio as the async runtime
- **Error handling**: Return `Result<T>` for fallible operations with custom error types
- **Configuration**: Use environment variables and CLI parameters only (no config files)

### Commit Messages

Write clear, concise commit messages:

```
feat: add Redis backend for activity queue

- Implement RedisQueue trait implementation
- Add connection pooling configuration
- Update documentation with Redis setup

Fixes #42
```

Use conventional commit prefixes:
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `test:` - Test additions or modifications
- `refactor:` - Code refactoring
- `perf:` - Performance improvements
- `chore:` - Maintenance tasks

## Testing

### Running Tests

```bash
# Run all tests
./scripts/test.sh

# Run with coverage
./scripts/test.sh --coverage

# Run specific test
cargo test test_name

# Run tests for a specific crate
cargo test -p kruxiaflow-core
```

### Writing Tests

- Write unit tests for new functionality
- Add integration tests for API endpoints and workflows
- Use meaningful test names that describe the scenario
- Tests should not depend on external services (only local Docker services)

### Test Database

Tests use a separate test database. Set up with:

```bash
./scripts/setup-dev-db.sh
```

## Documentation

### Code Documentation

- Document public APIs with rustdoc comments
- Include examples in documentation where helpful
- Update relevant docs when changing behavior

### Project Documentation

- Keep `docs/` directory up to date
- Use Mermaid for diagrams (renders in GitHub and mdBook)
- Update `docs/architecture.md` for architectural changes

## Community

### Getting Help

- **GitHub Discussions**: [Ask questions and share ideas](https://github.com/kruxia/kruxiaflow/discussions)
- **GitHub Issues**: [Report bugs and request features](https://github.com/kruxia/kruxiaflow/issues)

### Communication Guidelines

- Be respectful and constructive
- Search existing discussions before posting
- Provide context and be specific
- Help others when you can

## Recognition

Contributors are recognized in:
- Release notes for significant contributions
- The project README for major features

Thank you for contributing to Kruxia Flow!
