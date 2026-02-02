# Contributing to Kruxia Flow

Thank you for your interest in contributing to Kruxia Flow! This document provides guidelines and information for contributors.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How to Contribute](#how-to-contribute)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Documentation](#documentation)
- [Community](#community)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to [conduct@kruxia.com](mailto:conduct@kruxia.com).

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

1. Check [existing issues](https://github.com/kruxia/kruxiaflow/issues) first
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

- **GitHub Issues**: [Report bugs and request features](https://github.com/kruxia/kruxiaflow/issues)

### Communication Guidelines

- Be respectful and constructive
- Search existing issues and Discord before posting
- Provide context and be specific
- Help others when you can

## Recognition

Contributors are recognized in:
- Release notes for significant contributions
- The project README for major features

Thank you for contributing to Kruxia Flow!
