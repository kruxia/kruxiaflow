# Security Policy

## Supported Versions

StreamFlow is currently in active development. Security updates are provided for the following versions:

| Version | Supported          |
|--------:|--------------------|
| 0.3.x   | Yes                |
| < 0.3   | No                 |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, please send an email to [security@kruxia.com](mailto:security@kruxia.com) with:

1. **Description** - A clear description of the vulnerability
2. **Impact** - The potential impact and severity
3. **Steps to Reproduce** - Detailed steps to reproduce the issue
4. **Affected Versions** - Which versions are affected
5. **Suggested Fix** - If you have suggestions for how to fix it (optional)

### What to Expect

- **Acknowledgment**: We will acknowledge receipt of your report within 48 hours
- **Initial Assessment**: We will provide an initial assessment within 7 days
- **Resolution Timeline**: We aim to resolve critical vulnerabilities within 30 days
- **Disclosure**: We will coordinate with you on public disclosure timing

### Safe Harbor

We consider security research conducted in accordance with this policy to be:

- Authorized concerning any applicable anti-hacking laws
- Authorized concerning any relevant anti-circumvention laws
- Exempt from restrictions in our Terms of Service that would interfere with security research

We will not pursue civil action or initiate a complaint against researchers who:

- Engage in testing in accordance with this policy
- Report vulnerabilities in good faith
- Avoid privacy violations, destruction of data, and interruption of services
- Do not exploit vulnerabilities beyond what is necessary to demonstrate the issue

## Security Best Practices for StreamFlow Users

### Authentication

- **JWT Secrets**: Use strong, randomly generated JWT secrets (minimum 256 bits)
- **Token Expiration**: Configure appropriate token expiration times
- **API Keys**: Rotate API keys regularly

### Database

- **Connection Security**: Always use TLS for PostgreSQL connections in production
- **Credentials**: Use strong database passwords and follow the principle of least privilege
- **Network**: Restrict database access to only necessary hosts

### Deployment

- **HTTPS**: Always use HTTPS in production environments
- **Firewalls**: Restrict network access to StreamFlow services
- **Updates**: Keep StreamFlow and dependencies up to date
- **Secrets Management**: Use secure secrets management (environment variables, not config files)

### LLM API Keys

- **Storage**: Never commit API keys to version control
- **Rotation**: Rotate LLM provider API keys periodically
- **Scoping**: Use API keys with minimal necessary permissions
- **Monitoring**: Monitor API key usage for anomalies

### Docker Security

```yaml
# Example secure Docker Compose configuration
services:
  streamflow:
    image: ghcr.io/kruxia/streamflow:latest
    security_opt:
      - no-new-privileges:true
    read_only: true
    tmpfs:
      - /tmp
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - JWT_SECRET=${JWT_SECRET}
```

## Known Security Considerations

### Workflow Definitions

- Workflow definitions can execute HTTP requests to arbitrary URLs
- Database activity types can execute SQL queries
- Always validate and sanitize workflow definitions from untrusted sources

### Cost Control

- Budget limits help prevent runaway LLM costs but should not be relied upon as the sole protection
- Set up billing alerts with your LLM providers as an additional safeguard

### Semantic Cache

- Cached responses may contain sensitive information
- Configure appropriate TTLs and consider cache isolation for sensitive workloads

## Security Updates

Security updates are announced through:

- [GitHub Security Advisories](https://github.com/kruxia/streamflow/security/advisories)
- [Release Notes](https://github.com/kruxia/streamflow/releases)

## Contact

For security-related inquiries, contact [security@kruxia.com](mailto:security@kruxia.com).

For general questions, use [GitHub Discussions](https://github.com/kruxia/streamflow/discussions).
