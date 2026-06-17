# Example Policies Library

Production-ready Cedar policies for common enterprise governance scenarios.

## Categories

| Category | Description | Policies |
|----------|-------------|----------|
| [Security Baseline](security-baseline.md) | Core security policies every organization needs | 8 policies |
| [Dependency Management](dependency-management.md) | Control which libraries and packages can be used | 10 policies |
| [Architecture Constraints](architecture-constraints.md) | Enforce architectural decisions and patterns | 8 policies |
| [Code Quality](code-quality.md) | Maintain code quality standards | 7 policies |
| [Team Conventions](team-conventions.md) | Team-specific coding conventions | 6 policies |

## Quick Start

### Using Natural Language (Recommended)

```bash
# Aeterna translates natural language to Cedar
$ aeterna policy create "Block MySQL in this project"

Draft created: draft-no-mysql-1234
Translated to Cedar:
  forbid(principal, action == Action::"UseDependency", resource)
  when { resource.dependency == "mysql" };
```

### Using Pre-Built Policies

Copy any policy from this library:

```bash
# View policy
$ cat docs/examples/policies/security-baseline.cedar

# Import to your project
$ aeterna policy import docs/examples/policies/security-baseline.cedar --scope project
```

## Policy Structure

Each policy includes:

1. **Natural Language Description** - What the policy does in plain English
2. **Cedar Code** - The actual policy implementation
3. **Explanation** - How the policy works and why
4. **Use Cases** - When to apply this policy
5. **Customization** - How to adapt for your needs

## How Policies Work

### Severity Levels

| Severity | Effect | Use When |
|----------|--------|----------|
| `block` | Prevents the action entirely | Security violations, compliance requirements |
| `error` | Flags as error, may prevent deployment | Important violations that need fixing |
| `warn` | Logs warning, allows action | Best practices, migrations in progress |
| `info` | Informational only | Guidelines, suggestions |

### Scope Levels

```
Tenant (highest precedence)
    ↓ Mandatory policies flow down
Organization
    ↓ Department standards
Team
    ↓ Team conventions
Project (lowest precedence)
    ↓ Project-specific rules
```

### Merge Strategies

| Strategy | Behavior | Example |
|----------|----------|---------|
| `override` | Child replaces parent | Project uses different DB than team |
| `merge` | Combines both | Add project rules to team rules |
| `intersect` | Keeps only common | Stricter than both parent and child |

## Creating Custom Policies

### From Natural Language

```bash
$ aeterna policy create "Require all APIs to have OpenAPI documentation"

# Shows draft, allows review before submission
```

### From Templates

```bash
# List available templates
$ aeterna policy template list

# Create from template
$ aeterna policy template use require-file --args file=openapi.yaml,severity=warn
```

### Writing Cedar Directly

```cedar
// Policy: require-openapi-spec
// Scope: team
// Severity: warn

permit(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.files.contains("openapi.yaml") ||
  resource.files.contains("openapi.json")
};
```

## Best Practices

### 1. Start with Warnings

New policies should start as warnings to identify impact:

```bash
# First, deploy as warning
$ aeterna policy create "Require SECURITY.md" --severity warn

# After team adapts, upgrade to blocking
$ aeterna policy update require-security-md --severity block
```

### 2. Use Simulation Before Activation

```bash
$ aeterna policy simulate draft-no-mysql-1234

Simulation Results:
  Current project: PASS (no MySQL)
  Team projects: 2 would FAIL
    - legacy-api (uses mysql2)
    - data-sync (uses mysql)
```

### 3. Document Policy Rationale

```cedar
// Policy: no-moment-js
// Reason: moment.js is deprecated, use date-fns or dayjs
// Reference: ADR-015
// Migration guide: docs/migrations/moment-to-datefns.md
```

### 4. Plan for Exceptions

```cedar
// Allow specific exception with documentation
permit(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "moment" &&
  resource.exception_approved == true &&
  resource.exception_expires > context.now
};
```

## Index of All Policies

### Security

- `no-vulnerable-lodash` - Block lodash < 4.17.21 (CVE-2021-23337)
- `require-tls-1-3` - Enforce TLS 1.3+ for connections
- `no-eval` - Prevent eval() usage
- `require-security-md` - Require SECURITY.md file
- `no-hardcoded-secrets` - Block hardcoded credentials
- `require-input-validation` - Mandate input validation
- `require-auth-all-endpoints` - No anonymous API access
- `no-sql-string-concat` - Prevent SQL injection patterns

### Dependencies

- `no-mysql` - Use PostgreSQL instead of MySQL
- `no-moment` - Use date-fns or dayjs instead
- `no-lodash-full` - Use lodash-es or individual functions
- `require-opentelemetry` - Mandate tracing
- `no-deprecated-deps` - Block known deprecated packages
- `require-security-updates` - Enforce timely updates
- `pin-major-versions` - No floating major versions
- `no-install-scripts` - Block packages with install scripts
- `require-license-check` - Only allow approved licenses
- `max-bundle-size` - Limit dependency bundle size

### Architecture

- `require-strangler-facade` - Enforce strangler fig pattern
- `no-direct-db-access` - Use repository pattern
- `require-api-versioning` - Mandate API versions
- `no-circular-dependencies` - Prevent circular imports
- `require-interface-segregation` - Small, focused interfaces
- `require-error-boundaries` - Wrap components in error boundaries
- `max-function-length` - Limit function complexity
- `require-typed-errors` - Use typed errors, not panics

### Code Quality

- `require-tests` - Mandate test coverage
- `no-console-log` - Remove debug statements
- `require-jsdoc-public` - Document public APIs
- `no-any-type` - Avoid TypeScript `any`
- `require-error-handling` - No empty catch blocks
- `max-file-length` - Limit file size
- `require-code-review` - Mandate PR reviews

### Team Conventions

- `naming-convention-snake` - Use snake_case for APIs
- `naming-convention-camel` - Use camelCase for variables
- `require-changelog` - Mandate CHANGELOG.md
- `require-readme` - Require README.md
- `standard-commit-messages` - Conventional commits
- `require-pr-template` - Use PR templates
