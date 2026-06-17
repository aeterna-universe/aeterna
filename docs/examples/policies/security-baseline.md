# Security Baseline Policies

Essential security policies that every enterprise organization should implement.

---

## 1. Block Vulnerable Lodash

**Natural Language**: "Block lodash versions below 4.17.21 due to prototype pollution vulnerability"

**Cedar Policy**:
```cedar
// Policy: no-vulnerable-lodash
// Scope: tenant (mandatory)
// Severity: block
// Reference: CVE-2021-23337

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "lodash" &&
  resource.version < "4.17.21"
};
```

**Explanation**: 
Lodash versions before 4.17.21 contain a prototype pollution vulnerability (CVE-2021-23337) that can lead to remote code execution. This policy blocks any project from using vulnerable versions.

**Use Cases**:
- Tenant-wide security baseline
- Compliance requirements (SOC2, PCI-DSS)
- Supply chain security

**Customization**:
```cedar
// Also block lodash.template specifically (most affected)
forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "lodash.template" &&
  resource.version < "4.5.0"
};
```

---

## 2. Require TLS 1.3

**Natural Language**: "All network connections must use TLS 1.3 or higher"

**Cedar Policy**:
```cedar
// Policy: require-tls-1-3
// Scope: tenant (mandatory)
// Severity: block

forbid(
  principal,
  action == Action::"Configure",
  resource
)
when {
  resource.config_type == "tls" &&
  resource.tls_version < "1.3"
};
```

**Explanation**:
TLS 1.2 and below have known vulnerabilities. Modern security standards require TLS 1.3 for all network communications.

**Use Cases**:
- PCI-DSS compliance
- Data protection regulations
- Zero-trust architecture

**Customization**:
```cedar
// Allow TLS 1.2 only for specific legacy integrations with approval
permit(
  principal,
  action == Action::"Configure",
  resource
)
when {
  resource.config_type == "tls" &&
  resource.tls_version == "1.2" &&
  resource.legacy_exception_approved == true &&
  resource.exception_expires > context.now
};
```

---

## 3. No eval() Usage

**Natural Language**: "Prevent usage of eval() and similar dynamic code execution"

**Cedar Policy**:
```cedar
// Policy: no-eval
// Scope: tenant (mandatory)
// Severity: block

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.content.matches("eval\\s*\\(") ||
  resource.content.matches("new\\s+Function\\s*\\(") ||
  resource.content.matches("setTimeout\\s*\\(\\s*['\"]")
};
```

**Explanation**:
Dynamic code execution through `eval()`, `new Function()`, or string-based `setTimeout()` creates severe security risks including code injection attacks.

**Use Cases**:
- Application security
- Code review automation
- Preventing RCE vulnerabilities

**Customization**:
```cedar
// Exception for build tools that legitimately need eval
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches("scripts/build/.*") &&
  resource.reviewed_for_eval == true
};
```

---

## 4. Require SECURITY.md

**Natural Language**: "Every project must have a SECURITY.md file documenting security policies"

**Cedar Policy**:
```cedar
// Policy: require-security-md
// Scope: tenant
// Severity: warn (upgrade to block after grace period)

permit(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.files.contains("SECURITY.md")
};

// Warning if missing
forbid(
  principal,
  action == Action::"Deploy",
  resource
)
unless {
  resource.files.contains("SECURITY.md")
}
advice {
  message: "Missing SECURITY.md - create one documenting vulnerability reporting process"
};
```

**Explanation**:
SECURITY.md provides a standard location for security policies and vulnerability reporting instructions.

**Use Cases**:
- Open source projects
- Enterprise compliance
- Responsible disclosure

**Template**:
```markdown
# Security Policy

## Supported Versions
| Version | Supported |
|---------|-----------|
| 2.x     | ✅        |
| 1.x     | ❌        |

## Reporting a Vulnerability
Email security@example.com with:
- Description of vulnerability
- Steps to reproduce
- Impact assessment

Response within 48 hours.
```

---

## 5. No Hardcoded Secrets

**Natural Language**: "Block commits containing hardcoded credentials, API keys, or secrets"

**Cedar Policy**:
```cedar
// Policy: no-hardcoded-secrets
// Scope: tenant (mandatory)
// Severity: block

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  // API keys
  resource.content.matches("(?i)(api[_-]?key|apikey)\\s*[:=]\\s*['\"][a-zA-Z0-9]{20,}['\"]") ||
  // AWS credentials
  resource.content.matches("AKIA[0-9A-Z]{16}") ||
  // Private keys
  resource.content.matches("-----BEGIN (RSA |EC )?PRIVATE KEY-----") ||
  // Generic passwords
  resource.content.matches("(?i)password\\s*[:=]\\s*['\"][^'\"]{8,}['\"]") ||
  // JWT secrets
  resource.content.matches("(?i)jwt[_-]?secret\\s*[:=]\\s*['\"][^'\"]+['\"]")
};
```

**Explanation**:
Hardcoded secrets in source code are a major security risk. They can be exposed through version control history, logs, or error messages.

**Use Cases**:
- Preventing credential leaks
- CI/CD security
- Compliance requirements

**Customization**:
```cedar
// Exclude test files with obvious fake credentials
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*test.*") &&
  resource.content.matches("(?i)test[_-]?password|fake[_-]?key|mock[_-]?secret")
};
```

---

## 6. Require Input Validation

**Natural Language**: "All user input must be validated before processing"

**Cedar Policy**:
```cedar
// Policy: require-input-validation
// Scope: org
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  // Controller/handler files without validation imports
  resource.path.matches(".*(controller|handler|route|endpoint).*\\.(ts|js|rs)$") &&
  !resource.content.matches("(?i)(validate|validator|schema|zod|joi|yup|class-validator)")
};
```

**Explanation**:
Input validation prevents injection attacks, buffer overflows, and data corruption. All API endpoints should validate input before processing.

**Use Cases**:
- API security
- Data integrity
- OWASP compliance

**Customization**:
```cedar
// Specify your validation library
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*controller.*\\.ts$") &&
  !resource.content.matches("import.*from\\s*['\"]zod['\"]")
};
```

---

## 7. Require Authentication on All Endpoints

**Natural Language**: "No anonymous API access - all endpoints must require authentication"

**Cedar Policy**:
```cedar
// Policy: require-auth-all-endpoints
// Scope: org
// Severity: block

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.api_endpoints.any(endpoint =>
    !endpoint.requires_auth &&
    !endpoint.path.matches("^/(health|ready|metrics|docs|openapi)$")
  )
};
```

**Explanation**:
Every API endpoint should require authentication except for health checks, metrics, and documentation endpoints.

**Use Cases**:
- API security
- Zero-trust architecture
- Compliance requirements

**Customization**:
```cedar
// Allow specific public endpoints
permit(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.api_endpoints.all(endpoint =>
    endpoint.requires_auth ||
    endpoint.path in ["/health", "/ready", "/metrics", "/docs", "/v1/public/status"]
  )
};
```

---

## 8. No SQL String Concatenation

**Natural Language**: "Prevent SQL injection by blocking string concatenation in queries"

**Cedar Policy**:
```cedar
// Policy: no-sql-string-concat
// Scope: tenant (mandatory)
// Severity: block

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  // String concatenation in SQL
  resource.content.matches("(?i)(SELECT|INSERT|UPDATE|DELETE).*\\+.*\\$") ||
  // Template literals in SQL
  resource.content.matches("(?i)(SELECT|INSERT|UPDATE|DELETE).*\\$\\{") ||
  // Format strings in SQL (Rust/Python)
  resource.content.matches("(?i)(SELECT|INSERT|UPDATE|DELETE).*format!")
};
```

**Explanation**:
SQL injection remains one of the most common and dangerous vulnerabilities. Always use parameterized queries.

**Use Cases**:
- Database security
- OWASP Top 10 compliance
- Automated code review

**Allowed Patterns**:
```rust
// ✅ Good: Parameterized query
sqlx::query!("SELECT * FROM users WHERE id = $1", user_id)

// ❌ Bad: String concatenation
format!("SELECT * FROM users WHERE id = {}", user_id)
```

---

## Implementation Checklist

To implement this security baseline:

1. **Import as tenant-level policies**:
   ```bash
   $ aeterna policy import docs/examples/policies/security-baseline.md \
       --scope tenant \
       --mode mandatory
   ```

2. **Run simulation on all projects**:
   ```bash
   $ aeterna policy simulate security-baseline --scope tenant
   ```

3. **Review violations and plan remediation**:
   ```bash
   $ aeterna govern audit --type policy_violation --last 7d
   ```

4. **Gradually increase severity**:
   - Week 1-2: Deploy as warnings
   - Week 3-4: Upgrade to errors
   - Week 5+: Make blocking

5. **Monitor compliance**:
   ```bash
   $ aeterna status --scope tenant --include-compliance
   ```

---

## Related Policies

- [Dependency Management](dependency-management.md) - Control package usage
- [Architecture Constraints](architecture-constraints.md) - Enforce patterns
- [Code Quality](code-quality.md) - Maintain standards
