# Code Quality Policies

Maintain consistent code quality standards across your organization.

---

## 1. Require Tests

**Natural Language**: "All new code must have corresponding unit tests with minimum 80% coverage"

**Cedar Policy**:
```cedar
// Policy: require-tests
// Scope: tenant
// Severity: error

forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.changed_files.any(file =>
    file.path.matches(".*src/.*\\.(ts|js|rs)$") &&
    !file.path.matches(".*(test|spec|mock).*")
  ) &&
  !resource.changed_files.any(file =>
    file.path.matches(".*\\.(test|spec)\\.(ts|js|rs)$")
  )
}
advice {
  message: "PR adds source files without corresponding tests. Add tests in __tests__/ or *.test.ts"
};

// Enforce coverage threshold
forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.coverage_report.line_coverage < 80
}
advice {
  message: "Coverage is {resource.coverage_report.line_coverage}%. Minimum required: 80%"
};
```

**Explanation**:
Tests catch bugs early, enable safe refactoring, and document expected behavior. 80% coverage balances thoroughness with practicality.

**Use Cases**:
- Quality assurance
- Regression prevention
- Safe refactoring

**Test File Naming**:
```
src/
├── user.service.ts
├── user.service.test.ts      # Unit tests (co-located)
├── __tests__/
│   └── user.integration.ts   # Integration tests
└── __mocks__/
    └── user.repository.ts    # Mocks
```

**Customization**:
```cedar
// Lower threshold for legacy code being migrated
permit(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.project.tags.contains("legacy-migration") &&
  resource.coverage_report.line_coverage >= 60 &&
  resource.coverage_increasing == true
};
```

---

## 2. No Console Log

**Natural Language**: "Remove console.log statements before merging - use proper logging"

**Cedar Policy**:
```cedar
// Policy: no-console-log
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.(ts|js|tsx|jsx)$") &&
  !resource.path.matches(".*(test|spec|mock|debug).*") &&
  resource.content.matches("console\\.(log|debug|info)\\(")
}
advice {
  message: "Use logger instead of console.log. Import from '@/lib/logger'"
};

// Allow console.error/warn (they're often intentional)
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.content.matches("console\\.(error|warn)\\(") &&
  !resource.content.matches("console\\.(log|debug|info)\\(")
};
```

**Explanation**:
Console statements pollute logs, expose sensitive data, and indicate incomplete debugging. Production code should use structured logging.

**Use Cases**:
- Production readiness
- Log hygiene
- Security (preventing data leaks)

**Proper Logging**:
```typescript
// ❌ Bad: Console statements
console.log('User logged in:', user);
console.log('Processing order', { orderId, items });

// ✅ Good: Structured logging
import { logger } from '@/lib/logger';

logger.info('User logged in', { userId: user.id });
logger.info('Processing order', { orderId, itemCount: items.length });
```

**Logger Setup**:
```typescript
// lib/logger.ts
import pino from 'pino';

export const logger = pino({
  level: process.env.LOG_LEVEL || 'info',
  formatters: {
    level: (label) => ({ level: label }),
  },
  redact: ['password', 'token', 'secret'],  // Auto-redact sensitive fields
});
```

---

## 3. Require JSDoc on Public APIs

**Natural Language**: "All exported functions and classes must have JSDoc documentation"

**Cedar Policy**:
```cedar
// Policy: require-jsdoc-public
// Scope: team
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.(ts|js)$") &&
  resource.exports.any(exp =>
    exp.type in ["function", "class"] &&
    !exp.has_jsdoc
  )
}
advice {
  message: "Export '{exp.name}' is missing JSDoc. Add /** */ comment with description."
};

// Require examples for complex functions
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.exports.any(exp =>
    exp.type == "function" &&
    exp.param_count > 3 &&
    !exp.jsdoc.has_example
  )
}
advice {
  message: "Complex function '{exp.name}' should include @example in JSDoc"
};
```

**Explanation**:
Documentation enables IDE support, auto-generated API docs, and faster onboarding. JSDoc is particularly valuable for TypeScript projects.

**Use Cases**:
- API documentation
- IDE integration
- Developer experience

**JSDoc Template**:
```typescript
/**
 * Creates a new user account with the specified details.
 * 
 * @param data - The user registration data
 * @param options - Optional configuration for user creation
 * @returns The created user object with generated ID
 * @throws {ValidationError} If email format is invalid
 * @throws {DuplicateError} If email already exists
 * 
 * @example
 * ```typescript
 * const user = await createUser(
 *   { email: 'test@example.com', name: 'Test' },
 *   { sendWelcomeEmail: true }
 * );
 * console.log(user.id); // 'usr_abc123'
 * ```
 */
export async function createUser(
  data: UserRegistration,
  options?: CreateUserOptions
): Promise<User> {
  // Implementation
}
```

---

## 4. No Any Type

**Natural Language**: "Avoid TypeScript 'any' type - use proper types or 'unknown'"

**Cedar Policy**:
```cedar
// Policy: no-any-type
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.tsx?$") &&
  !resource.path.matches(".*(test|spec|mock|\\.d\\.ts).*") &&
  resource.content.matches(": any[^a-zA-Z]|<any>|as any")
}
advice {
  message: "Avoid 'any' type. Use specific types, generics, or 'unknown' with type guards."
};

// Also catch implicit any from tsconfig
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.file == "tsconfig.json" &&
  resource.content.matches("\"noImplicitAny\":\\s*false")
};
```

**Explanation**:
The `any` type defeats TypeScript's purpose. It disables type checking and can hide bugs. Use `unknown` with type guards for truly dynamic data.

**Use Cases**:
- Type safety
- Bug prevention
- Better IDE support

**Alternatives to Any**:
```typescript
// ❌ Bad: any
function process(data: any) {
  return data.value;  // No type checking!
}

// ✅ Good: Specific type
function process(data: { value: string }) {
  return data.value;
}

// ✅ Good: Generic
function process<T extends { value: unknown }>(data: T) {
  return data.value;
}

// ✅ Good: unknown with type guard
function process(data: unknown) {
  if (isValidData(data)) {
    return data.value;
  }
  throw new ValidationError('Invalid data');
}

function isValidData(data: unknown): data is { value: string } {
  return typeof data === 'object' && data !== null && 'value' in data;
}
```

---

## 5. Require Error Handling

**Natural Language**: "No empty catch blocks - all errors must be handled or re-thrown"

**Cedar Policy**:
```cedar
// Policy: require-error-handling
// Scope: tenant
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.(ts|js|rs)$") &&
  resource.content.matches("catch\\s*\\([^)]*\\)\\s*\\{\\s*\\}")
}
advice {
  message: "Empty catch block detected. Handle the error, log it, or re-throw."
};

// Also check for swallowed errors (catch with no action)
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.catch_blocks.any(block =>
    block.body.line_count == 0 ||
    (block.body.line_count == 1 && block.body.matches("//.*ignore"))
  )
};
```

**Explanation**:
Empty catch blocks silently swallow errors, making debugging nearly impossible. Every error should be logged, handled, or re-thrown with context.

**Use Cases**:
- Debugging
- Error visibility
- Reliability

**Proper Error Handling**:
```typescript
// ❌ Bad: Swallowed error
try {
  await riskyOperation();
} catch (e) {
  // Silent failure - impossible to debug
}

// ❌ Bad: Logged but not handled
try {
  await riskyOperation();
} catch (e) {
  console.log(e);  // Logs but doesn't handle
}

// ✅ Good: Proper handling
try {
  await riskyOperation();
} catch (e) {
  logger.error('Risky operation failed', { error: e, context });
  throw new OperationError('Failed to complete operation', { cause: e });
}

// ✅ Good: Graceful degradation
try {
  data = await fetchFromPrimary();
} catch (e) {
  logger.warn('Primary fetch failed, using fallback', { error: e });
  data = await fetchFromFallback();
}
```

---

## 6. Max File Length

**Natural Language**: "Files should not exceed 500 lines - split into modules"

**Cedar Policy**:
```cedar
// Policy: max-file-length
// Scope: team
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.line_count > 500 &&
  !resource.path.matches(".*(test|spec|mock|\\.generated\\.).*")
}
advice {
  message: "File is {resource.line_count} lines. Consider splitting into smaller modules."
};

// Stricter for specific file types
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.(tsx|jsx)$") &&  // React components
  resource.line_count > 300
}
advice {
  message: "Component file exceeds 300 lines. Extract sub-components or hooks."
};
```

**Explanation**:
Large files are hard to navigate, understand, and maintain. They often indicate that a module has too many responsibilities.

**Use Cases**:
- Code organization
- Maintainability
- Collaboration (reduce merge conflicts)

**Splitting Strategies**:
```
// Before: 800-line user.service.ts
src/
└── user.service.ts

// After: Split by responsibility
src/
├── user/
│   ├── index.ts           # Public exports
│   ├── user.types.ts      # Type definitions
│   ├── user.service.ts    # Core CRUD operations
│   ├── user.auth.ts       # Authentication logic
│   ├── user.validation.ts # Validation rules
│   └── user.notifications.ts # Email/SMS logic
```

**Customization**:
```cedar
// Allow longer files for specific patterns
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.line_count > 500 &&
  (
    resource.path.matches(".*\\.stories\\.tsx$") ||  // Storybook
    resource.path.matches(".*schema\\.ts$") ||       // Schema definitions
    resource.path.matches(".*constants\\.ts$")       // Constants files
  )
};
```

---

## 7. Require Code Review

**Natural Language**: "All PRs must have at least one approval from a team member"

**Cedar Policy**:
```cedar
// Policy: require-code-review
// Scope: tenant
// Severity: block

forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.approvals < 1
}
advice {
  message: "PR requires at least 1 approval before merging"
};

// Require 2 approvals for critical paths
forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.changed_files.any(file =>
    file.path.matches(".*(auth|security|payment|billing).*")
  ) &&
  resource.approvals < 2
}
advice {
  message: "Changes to critical paths require 2 approvals"
};

// Prevent self-approval
forbid(
  principal,
  action == Action::"Approve",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.author == principal.id
};
```

**Explanation**:
Code review catches bugs, shares knowledge, and maintains code quality. Critical paths (auth, payments) require additional scrutiny.

**Use Cases**:
- Quality assurance
- Knowledge sharing
- Compliance requirements

**Review Requirements by Path**:
| Path Pattern | Required Approvals | Reviewer Requirements |
|--------------|-------------------|----------------------|
| `src/**` | 1 | Any team member |
| `**/auth/**` | 2 | At least 1 security champion |
| `**/billing/**` | 2 | At least 1 platform engineer |
| `infrastructure/**` | 2 | At least 1 DevOps engineer |
| `**/migrations/**` | 2 | At least 1 database owner |

**Customization**:
```cedar
// Allow single-person teams to merge without approval
permit(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.team.member_count == 1 &&
  resource.ci_checks_passed == true
};

// Auto-approve bot PRs for dependency updates
permit(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.author in ["dependabot[bot]", "renovate[bot]"] &&
  resource.ci_checks_passed == true &&
  resource.changed_files.all(file =>
    file.path.matches("(package\\.json|Cargo\\.toml|.*\\.lock)")
  )
};
```

---

## Implementation Checklist

To implement code quality policies:

1. **Configure tooling**:
   ```json
   // package.json
   {
     "scripts": {
       "lint": "eslint src/",
       "test": "jest --coverage",
       "type-check": "tsc --noEmit"
     }
   }
   ```

2. **Set up CI checks**:
   ```yaml
   # .github/workflows/ci.yml
   - name: Lint
     run: npm run lint
   
   - name: Type Check
     run: npm run type-check
   
   - name: Test with Coverage
     run: npm run test -- --coverageThreshold='{"global":{"lines":80}}'
   ```

3. **Configure branch protection**:
   ```bash
   $ gh api repos/{owner}/{repo}/branches/main/protection -X PUT \
       -f required_pull_request_reviews.required_approving_review_count=1
   ```

4. **Import policies**:
   ```bash
   $ aeterna policy import code-quality.md \
       --scope org \
       --mode enforce
   ```

5. **Track metrics**:
   ```bash
   $ aeterna govern audit --type code_quality --trend 30d
   
   Code Quality Trends (30 days):
     Coverage: 78% → 85% (+7%)
     Any types: 45 → 12 (-73%)
     Console logs: 23 → 3 (-87%)
     Empty catches: 8 → 0 (-100%)
   ```

---

## ESLint Integration

Many of these policies can be enforced via ESLint:

```json
// .eslintrc.json
{
  "rules": {
    "no-console": "error",
    "@typescript-eslint/no-explicit-any": "error",
    "no-empty": ["error", { "allowEmptyCatch": false }],
    "jsdoc/require-jsdoc": ["warn", {
      "require": {
        "FunctionDeclaration": true,
        "ClassDeclaration": true
      },
      "contexts": ["ExportNamedDeclaration"]
    }],
    "max-lines": ["warn", { "max": 500, "skipBlankLines": true }],
    "max-lines-per-function": ["warn", { "max": 50 }]
  }
}
```

---

## Related Policies

- [Security Baseline](security-baseline.md) - Core security policies
- [Dependency Management](dependency-management.md) - Control packages
- [Architecture Constraints](architecture-constraints.md) - Enforce patterns
- [Team Conventions](team-conventions.md) - Team-specific rules
