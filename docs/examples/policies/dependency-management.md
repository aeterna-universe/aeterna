# Dependency Management Policies

Control which libraries and packages can be used across your organization.

---

## 1. No MySQL

**Natural Language**: "Use PostgreSQL instead of MySQL for all new projects"

**Cedar Policy**:
```cedar
// Policy: no-mysql
// Scope: tenant
// Severity: block
// Reference: ADR-042

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency in ["mysql", "mysql2", "mysqlclient", "pymysql", "mysql-connector-python"]
};
```

**Explanation**:
Standardizing on PostgreSQL reduces operational complexity and enables advanced features like JSONB and full-text search.

**Use Cases**:
- Database standardization
- Reducing operational overhead
- Enabling advanced PostgreSQL features

**Customization**:
```cedar
// Allow MySQL only for legacy migration projects
permit(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency in ["mysql", "mysql2"] &&
  resource.project.tags.contains("legacy-migration") &&
  resource.migration_deadline > context.now
};
```

---

## 2. No Moment.js

**Natural Language**: "Use date-fns or dayjs instead of moment.js - moment is deprecated"

**Cedar Policy**:
```cedar
// Policy: no-moment
// Scope: tenant
// Severity: error
// Reference: https://momentjs.com/docs/#/-project-status/

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "moment" ||
  resource.dependency.startsWith("moment-")
};
```

**Explanation**:
Moment.js is in maintenance mode and not recommended for new projects. It's also very large (300KB+) and mutable. Modern alternatives like date-fns are tree-shakeable and immutable.

**Use Cases**:
- Reducing bundle size
- Using maintained libraries
- Modernizing date handling

**Migration Guide**:
```javascript
// Before (moment)
moment().add(7, 'days').format('YYYY-MM-DD')

// After (date-fns)
import { addDays, format } from 'date-fns'
format(addDays(new Date(), 7), 'yyyy-MM-dd')

// After (dayjs)
import dayjs from 'dayjs'
dayjs().add(7, 'day').format('YYYY-MM-DD')
```

**Customization**:
```cedar
// Grace period for existing projects
permit(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "moment" &&
  resource.project.created_before < "2023-01-01" &&
  resource.migration_planned == true
};
```

---

## 3. No Full Lodash

**Natural Language**: "Use lodash-es or individual lodash functions, not the full bundle"

**Cedar Policy**:
```cedar
// Policy: no-lodash-full
// Scope: org
// Severity: warn

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency == "lodash" &&
  !resource.import_style == "cherry-pick"
}
advice {
  message: "Use 'lodash-es' or individual imports like 'lodash/debounce' for better tree-shaking"
};
```

**Explanation**:
The full lodash bundle is 70KB+ minified. Using lodash-es or cherry-picked imports enables tree-shaking and dramatically reduces bundle size.

**Use Cases**:
- Frontend bundle optimization
- Performance improvement
- Build time reduction

**Allowed Patterns**:
```javascript
// ✅ Good: Individual imports
import debounce from 'lodash/debounce'
import { debounce } from 'lodash-es'

// ❌ Bad: Full bundle
import _ from 'lodash'
import { debounce } from 'lodash'  // Still imports full bundle!
```

---

## 4. Require OpenTelemetry

**Natural Language**: "All services must include OpenTelemetry for distributed tracing"

**Cedar Policy**:
```cedar
// Policy: require-opentelemetry
// Scope: org (Platform Engineering)
// Severity: warn

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.type == "service" &&
  !resource.dependencies.any(dep => 
    dep.name.contains("opentelemetry") ||
    dep.name.contains("otel")
  )
}
advice {
  message: "Add opentelemetry-sdk to enable distributed tracing. See: docs/observability/tracing.md"
};
```

**Explanation**:
Distributed tracing is essential for debugging microservices. OpenTelemetry provides vendor-neutral instrumentation.

**Use Cases**:
- Microservices observability
- Performance debugging
- Incident response

**Implementation Example**:
```rust
// Rust: Add to Cargo.toml
[dependencies]
opentelemetry = "0.21"
opentelemetry-otlp = "0.14"
tracing-opentelemetry = "0.22"

// TypeScript: Add to package.json
"@opentelemetry/api": "^1.7.0",
"@opentelemetry/sdk-node": "^0.45.0",
"@opentelemetry/auto-instrumentations-node": "^0.39.0"
```

---

## 5. No Deprecated Dependencies

**Natural Language**: "Block packages that are officially deprecated or unmaintained"

**Cedar Policy**:
```cedar
// Policy: no-deprecated-deps
// Scope: tenant
// Severity: error

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.dependency in [
    "request",           // Deprecated 2020
    "node-uuid",         // Use 'uuid' instead
    "istanbul",          // Use 'nyc' instead
    "coffee-script",     // Deprecated
    "tslint",            // Use eslint instead
    "node-sass",         // Use sass (dart-sass)
    "faker",             // Compromised, use @faker-js/faker
    "colors",            // Compromised
    "left-pad"           // Historical, use String.padStart
  ]
};
```

**Explanation**:
Deprecated packages don't receive security updates and may have known vulnerabilities. Using maintained alternatives ensures security and compatibility.

**Use Cases**:
- Security compliance
- Supply chain security
- Technical debt prevention

**Migration Mapping**:
| Deprecated | Replacement |
|------------|-------------|
| `request` | `node-fetch`, `axios`, `got` |
| `node-uuid` | `uuid` |
| `istanbul` | `nyc` or `c8` |
| `tslint` | `eslint` + `@typescript-eslint` |
| `node-sass` | `sass` (dart-sass) |
| `faker` | `@faker-js/faker` |

---

## 6. Require Security Updates

**Natural Language**: "Dependencies with known vulnerabilities must be updated within 30 days"

**Cedar Policy**:
```cedar
// Policy: require-security-updates
// Scope: tenant
// Severity: block (critical), error (high), warn (medium)

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.vulnerabilities.any(vuln =>
    vuln.severity == "critical" &&
    vuln.discovered_days_ago > 7
  )
};

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.vulnerabilities.any(vuln =>
    vuln.severity == "high" &&
    vuln.discovered_days_ago > 14
  )
};

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.vulnerabilities.any(vuln =>
    vuln.severity == "medium" &&
    vuln.discovered_days_ago > 30
  )
}
advice {
  message: "Run 'npm audit fix' or 'cargo audit fix' to resolve vulnerabilities"
};
```

**Explanation**:
Timely patching of security vulnerabilities is essential for maintaining security posture. This policy enforces SLAs based on severity.

**Use Cases**:
- SOC2 compliance
- Security SLAs
- Risk management

**Customization**:
```cedar
// Extended deadline for complex updates requiring testing
permit(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.vulnerabilities.all(vuln =>
    vuln.exception_approved == true &&
    vuln.exception_reason != "" &&
    vuln.exception_expires > context.now
  )
};
```

---

## 7. Pin Major Versions

**Natural Language**: "No floating major versions - all dependencies must have pinned major version"

**Cedar Policy**:
```cedar
// Policy: pin-major-versions
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.file == "package.json" &&
  resource.content.matches("\"[*x]\"") ||
  resource.content.matches("\">=[0-9]\"") ||
  resource.content.matches("\"latest\"")
};

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.file == "Cargo.toml" &&
  resource.content.matches("\"\\*\"")
};
```

**Explanation**:
Floating major versions can introduce breaking changes unexpectedly. Pinning at least the major version ensures reproducible builds.

**Use Cases**:
- Build reproducibility
- Preventing surprise breakages
- CI/CD stability

**Allowed Patterns**:
```json
{
  "dependencies": {
    "react": "^18.2.0",    // ✅ Good: Caret allows minor/patch
    "lodash": "~4.17.21",  // ✅ Good: Tilde allows patch only
    "axios": "1.6.0",      // ✅ Good: Exact version
    "express": "*",        // ❌ Bad: Floating
    "moment": "latest"     // ❌ Bad: Latest tag
  }
}
```

---

## 8. No Install Scripts

**Natural Language**: "Block packages that run arbitrary scripts during installation"

**Cedar Policy**:
```cedar
// Policy: no-install-scripts
// Scope: tenant
// Severity: warn

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.has_install_scripts == true &&
  !resource.dependency in context.allowed_install_scripts
}
advice {
  message: "Package runs scripts during install. Review security implications or use --ignore-scripts"
};
```

**Explanation**:
Install scripts (postinstall, preinstall) can execute arbitrary code during `npm install`. This is a supply chain attack vector.

**Use Cases**:
- Supply chain security
- CI/CD hardening
- Preventing malicious code execution

**Mitigation**:
```bash
# Disable install scripts globally
npm config set ignore-scripts true

# Or per-install
npm install --ignore-scripts

# Then explicitly run trusted scripts
npm rebuild
```

**Customization**:
```cedar
// Allowlist for trusted packages that need install scripts
permit(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.has_install_scripts == true &&
  resource.dependency in [
    "esbuild",      // Native binary download
    "sharp",        // Image processing native deps
    "sqlite3",      // Native compilation
    "@prisma/client" // Prisma engine download
  ]
};
```

---

## 9. Require License Check

**Natural Language**: "Only allow dependencies with approved open source licenses"

**Cedar Policy**:
```cedar
// Policy: require-license-check
// Scope: tenant
// Severity: block

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  !resource.license in [
    "MIT",
    "Apache-2.0", 
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "0BSD",
    "Unlicense",
    "CC0-1.0",
    "MPL-2.0"      // Weak copyleft, generally OK
  ]
}
advice {
  message: "License not in approved list. Contact legal@example.com for approval."
};

// Explicitly block strong copyleft for proprietary products
forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.license in ["GPL-2.0", "GPL-3.0", "AGPL-3.0", "LGPL-3.0"] &&
  resource.project.distribution == "proprietary"
};
```

**Explanation**:
License compliance is essential for legal protection. Some licenses (GPL, AGPL) have copyleft requirements that may conflict with proprietary software.

**Use Cases**:
- Legal compliance
- Open source governance
- Risk management

**License Categories**:
| Category | Licenses | Typical Use |
|----------|----------|-------------|
| Permissive | MIT, Apache-2.0, BSD | ✅ All uses |
| Weak Copyleft | LGPL, MPL | ⚠️ Usually OK, check |
| Strong Copyleft | GPL, AGPL | ❌ Review with legal |
| Unknown | No license | ❌ Avoid |

---

## 10. Max Bundle Size

**Natural Language**: "No single dependency should add more than 100KB to the bundle"

**Cedar Policy**:
```cedar
// Policy: max-bundle-size
// Scope: team (Frontend)
// Severity: warn

forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.bundle_size_kb > 100 &&
  resource.project.type == "frontend"
}
advice {
  message: "Dependency adds {resource.bundle_size_kb}KB to bundle. Consider alternatives or lazy loading."
};

// Hard block for extremely large dependencies
forbid(
  principal,
  action == Action::"UseDependency",
  resource
)
when {
  resource.bundle_size_kb > 500 &&
  resource.project.type == "frontend"
};
```

**Explanation**:
Large dependencies hurt page load performance. Monitoring bundle size prevents performance regression.

**Use Cases**:
- Frontend performance
- Mobile optimization
- Core Web Vitals

**Tools for Analysis**:
```bash
# Analyze bundle impact before adding
npx bundle-phobia-cli lodash

# Visualize current bundle
npx webpack-bundle-analyzer

# Check size during development
npm install --save-dev size-limit
```

**Large Dependency Alternatives**:
| Heavy | Size | Lighter Alternative | Size |
|-------|------|---------------------|------|
| moment | 300KB | date-fns | 30KB |
| lodash | 70KB | lodash-es (tree-shake) | 10KB |
| chart.js | 200KB | lightweight-charts | 40KB |
| aws-sdk | 3MB+ | @aws-sdk/client-* | varies |

---

## Implementation Checklist

To implement dependency management policies:

1. **Audit current dependencies**:
   ```bash
   $ aeterna policy simulate dependency-management --scope org
   
   Simulation Results:
     Projects scanned: 47
     Violations found: 23
       - 8 using moment.js
       - 5 using full lodash
       - 3 missing opentelemetry
       - 7 with floating versions
   ```

2. **Plan migration timeline**:
   ```bash
   $ aeterna policy create migration-plan \
       --violations 23 \
       --deadline 90d
   ```

3. **Deploy as warnings first**:
   ```bash
   $ aeterna policy import dependency-management.md \
       --scope org \
       --severity warn
   ```

4. **Track remediation progress**:
   ```bash
   $ aeterna govern audit --type dependency_violation --trend 30d
   ```

5. **Escalate severity**:
   ```bash
   # After 30 days
   $ aeterna policy update no-moment --severity error
   
   # After 60 days  
   $ aeterna policy update no-moment --severity block
   ```

---

## Related Policies

- [Security Baseline](security-baseline.md) - Core security policies
- [Architecture Constraints](architecture-constraints.md) - Enforce patterns
- [Code Quality](code-quality.md) - Maintain standards
