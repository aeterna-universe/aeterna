# Architecture Constraints Policies

Enforce architectural decisions and design patterns across your organization.

---

## 1. Require Strangler Facade

**Natural Language**: "All legacy system integrations must use the Strangler Facade pattern"

**Cedar Policy**:
```cedar
// Policy: require-strangler-facade
// Scope: org (Platform Engineering)
// Severity: block
// Reference: ADR-001

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.project.tags.contains("legacy-migration") &&
  resource.imports_from.any(import =>
    import.source.matches("legacy-.*") &&
    !import.through_facade == true
  )
};

// Require facade registration for new integrations
forbid(
  principal,
  action == Action::"CreateIntegration",
  resource
)
when {
  resource.target.type == "legacy" &&
  !resource.facade_registered == true
}
advice {
  message: "Register facade in services/facades/ before integrating with legacy system"
};
```

**Explanation**:
The Strangler Facade pattern enables gradual migration from legacy systems by creating a facade that routes requests to either the old or new implementation. This prevents direct coupling to legacy code.

**Use Cases**:
- Legacy modernization
- Platform migrations
- Risk reduction during rewrites

**Pattern Diagram**:
```
┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
│   Client    │────►│ Strangler Facade │────►│ New Service  │
└─────────────┘     │                 │     └──────────────┘
                    │  Routing Logic  │
                    │                 │     ┌──────────────┐
                    │                 │────►│Legacy System │
                    └─────────────────┘     └──────────────┘
```

**Customization**:
```cedar
// Allow direct legacy access for monitoring/debugging tools
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches("tools/(monitoring|debugging)/.*") &&
  resource.imports_from.all(import =>
    import.purpose == "read-only"
  )
};
```

---

## 2. No Direct Database Access

**Natural Language**: "Services must use the repository pattern - no direct SQL queries in controllers"

**Cedar Policy**:
```cedar
// Policy: no-direct-db-access
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  // Controller/handler files with direct DB imports
  resource.path.matches(".*(controller|handler|route|endpoint|resolver).*\\.(ts|js|rs)$") &&
  (
    resource.content.matches("import.*from\\s*['\"]pg['\"]") ||
    resource.content.matches("import.*from\\s*['\"]mysql") ||
    resource.content.matches("use\\s+sqlx::") ||
    resource.content.matches("import.*from\\s*['\"]@prisma/client['\"]")
  )
}
advice {
  message: "Use repository pattern. Import from '../repositories/' instead of direct DB access."
};
```

**Explanation**:
The repository pattern provides an abstraction layer between business logic and data access. This enables easier testing, database changes, and separation of concerns.

**Use Cases**:
- Clean architecture
- Testability improvement
- Database abstraction

**Correct Structure**:
```
src/
├── controllers/      # HTTP handling only
│   └── user.controller.ts
├── services/         # Business logic
│   └── user.service.ts
├── repositories/     # Data access
│   └── user.repository.ts
└── entities/         # Domain models
    └── user.entity.ts
```

**Example**:
```typescript
// ❌ Bad: Direct DB in controller
class UserController {
  async getUser(id: string) {
    return await prisma.user.findUnique({ where: { id } });
  }
}

// ✅ Good: Repository pattern
class UserController {
  constructor(private userRepo: UserRepository) {}
  
  async getUser(id: string) {
    return await this.userRepo.findById(id);
  }
}
```

---

## 3. Require API Versioning

**Natural Language**: "All public APIs must include version in the URL path"

**Cedar Policy**:
```cedar
// Policy: require-api-versioning
// Scope: tenant
// Severity: block

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.type == "api" &&
  resource.visibility == "public" &&
  !resource.endpoints.all(endpoint =>
    endpoint.path.matches("^/v[0-9]+/.*")
  )
};

// Require version documentation
forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.type == "api" &&
  resource.visibility == "public" &&
  !resource.files.contains("API_VERSIONING.md")
}
advice {
  message: "Document API versioning strategy in API_VERSIONING.md"
};
```

**Explanation**:
API versioning prevents breaking changes from affecting clients. URL-based versioning (v1, v2) is explicit and cache-friendly.

**Use Cases**:
- API stability
- Client compatibility
- Gradual deprecation

**Versioning Strategies**:
| Strategy | Example | Pros | Cons |
|----------|---------|------|------|
| URL Path | `/v1/users` | Clear, cacheable | URL changes |
| Header | `Accept: application/vnd.api+json;version=1` | Clean URLs | Hidden |
| Query | `/users?version=1` | Simple | Caching issues |

**Recommended: URL Path**
```yaml
# openapi.yaml
servers:
  - url: https://api.example.com/v1
    description: Version 1 (current)
  - url: https://api.example.com/v2
    description: Version 2 (beta)
```

---

## 4. No Circular Dependencies

**Natural Language**: "Prevent circular imports between modules"

**Cedar Policy**:
```cedar
// Policy: no-circular-dependencies
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.module_graph.has_cycles == true
}
advice {
  message: "Circular dependency detected: {resource.module_graph.cycle_path}"
};

// Also check at package level
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.file == "package.json" &&
  resource.workspace_deps.has_cycles == true
};
```

**Explanation**:
Circular dependencies create tight coupling, make testing difficult, and can cause runtime issues. They often indicate architectural problems.

**Use Cases**:
- Code maintainability
- Build performance
- Clear module boundaries

**Detection Tools**:
```bash
# TypeScript/JavaScript
npx madge --circular src/

# Rust
cargo +nightly udeps --all

# Python
pydeps --show-cycles src/
```

**Resolution Patterns**:
```typescript
// Problem: A imports B, B imports A
// file: a.ts
import { B } from './b'  // B needs A!

// Solution 1: Extract shared code
// file: shared.ts
export interface SharedType { ... }

// Solution 2: Dependency injection
// file: a.ts
class A {
  constructor(private b: BInterface) {}
}

// Solution 3: Event-based communication
// file: a.ts
eventBus.emit('a:completed', data)
// file: b.ts
eventBus.on('a:completed', handler)
```

---

## 5. Require Interface Segregation

**Natural Language**: "Interfaces should be small and focused - no god interfaces"

**Cedar Policy**:
```cedar
// Policy: require-interface-segregation
// Scope: team
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.interfaces.any(iface =>
    iface.method_count > 7
  )
}
advice {
  message: "Interface has too many methods ({count}). Split into smaller, focused interfaces."
};

// Also check trait implementations in Rust
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.rs$") &&
  resource.traits.any(trait =>
    trait.method_count > 10 &&
    !trait.name.matches(".*Builder$")  // Builders are OK
  )
};
```

**Explanation**:
The Interface Segregation Principle (ISP) states that clients shouldn't depend on interfaces they don't use. Large interfaces force unnecessary dependencies.

**Use Cases**:
- SOLID principles
- Reduced coupling
- Better testability

**Example**:
```typescript
// ❌ Bad: God interface
interface UserService {
  getUser(id: string): User
  createUser(data: UserData): User
  updateUser(id: string, data: UserData): User
  deleteUser(id: string): void
  sendEmail(userId: string, email: Email): void
  resetPassword(userId: string): void
  generateReport(userId: string): Report
  importUsers(file: File): User[]
  exportUsers(): File
  validateUser(data: UserData): ValidationResult
}

// ✅ Good: Segregated interfaces
interface UserReader {
  getUser(id: string): User
}

interface UserWriter {
  createUser(data: UserData): User
  updateUser(id: string, data: UserData): User
  deleteUser(id: string): void
}

interface UserNotifier {
  sendEmail(userId: string, email: Email): void
  resetPassword(userId: string): void
}

interface UserBulkOperations {
  importUsers(file: File): User[]
  exportUsers(): File
}
```

---

## 6. Require Error Boundaries

**Natural Language**: "React components must be wrapped in error boundaries at route level"

**Cedar Policy**:
```cedar
// Policy: require-error-boundaries
// Scope: team (Frontend)
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*/pages/.*\\.(tsx|jsx)$") &&
  !resource.content.matches("ErrorBoundary")
}
advice {
  message: "Page components should be wrapped in ErrorBoundary. See: docs/frontend/error-handling.md"
};

// Require ErrorBoundary component exists
forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.type == "frontend" &&
  !resource.files.any(f => f.matches(".*ErrorBoundary.*\\.(tsx|jsx)$"))
};
```

**Explanation**:
Error boundaries catch JavaScript errors in React component trees, preventing entire app crashes. They're essential for production stability.

**Use Cases**:
- Production stability
- Graceful degradation
- Error reporting

**Implementation**:
```tsx
// components/ErrorBoundary.tsx
class ErrorBoundary extends React.Component<Props, State> {
  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    // Log to monitoring service
    errorReporter.captureException(error, { extra: errorInfo });
  }

  render() {
    if (this.state.hasError) {
      return <ErrorFallback error={this.state.error} />;
    }
    return this.props.children;
  }
}

// Usage in routes
<Route path="/dashboard">
  <ErrorBoundary fallback={<DashboardError />}>
    <Dashboard />
  </ErrorBoundary>
</Route>
```

---

## 7. Max Function Length

**Natural Language**: "Functions should not exceed 50 lines - extract helper functions for complex logic"

**Cedar Policy**:
```cedar
// Policy: max-function-length
// Scope: team
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.functions.any(fn =>
    fn.line_count > 50 &&
    !fn.name.matches(".*test.*|.*spec.*")  // Exclude tests
  )
}
advice {
  message: "Function '{fn.name}' is {fn.line_count} lines. Consider extracting helper functions."
};

// Stricter for controllers/handlers
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*(controller|handler).*") &&
  resource.functions.any(fn => fn.line_count > 30)
};
```

**Explanation**:
Long functions are hard to understand, test, and maintain. They often violate single responsibility and can hide bugs.

**Use Cases**:
- Code readability
- Maintainability
- Testability

**Refactoring Strategies**:
```typescript
// ❌ Bad: 80-line function
async function processOrder(order: Order) {
  // Validate order (15 lines)
  // Calculate totals (20 lines)
  // Apply discounts (15 lines)
  // Process payment (20 lines)
  // Send notifications (10 lines)
}

// ✅ Good: Extracted functions
async function processOrder(order: Order) {
  const validated = await validateOrder(order);
  const totals = calculateTotals(validated);
  const discounted = applyDiscounts(totals);
  const payment = await processPayment(discounted);
  await sendNotifications(payment);
  return payment;
}

// Each helper is focused and testable
function calculateTotals(order: ValidatedOrder): OrderTotals {
  // 15 focused lines
}
```

---

## 8. Require Typed Errors

**Natural Language**: "Use Result types with typed errors, not panics or thrown exceptions"

**Cedar Policy**:
```cedar
// Policy: require-typed-errors
// Scope: org (Platform Engineering)
// Severity: error

// Rust: No unwrap in production code
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.rs$") &&
  !resource.path.matches(".*(test|spec|mock).*") &&
  resource.content.matches("\\.unwrap\\(\\)") &&
  !resource.content.matches("// SAFETY:")  // Allow documented unwraps
};

// Rust: No panic! macro
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.rs$") &&
  !resource.path.matches(".*(test|spec).*") &&
  resource.content.matches("panic!\\(")
};

// TypeScript: Require error types
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.ts$") &&
  resource.content.matches("throw\\s+new\\s+Error\\(") &&
  !resource.content.matches("throw\\s+new\\s+[A-Z][a-zA-Z]+Error\\(")
}
advice {
  message: "Use typed error classes instead of generic Error. See: src/errors/"
};
```

**Explanation**:
Typed errors enable exhaustive error handling, better tooling support, and clearer contracts. They make error recovery explicit and predictable.

**Use Cases**:
- Error handling
- API reliability
- Debugging

**Rust Example**:
```rust
// ❌ Bad: Panics and unwraps
fn process(input: &str) -> Output {
    let parsed = input.parse::<i32>().unwrap();  // Panic on invalid!
    if parsed < 0 {
        panic!("Negative not allowed");
    }
    Output { value: parsed }
}

// ✅ Good: Typed Result errors
#[derive(Debug, thiserror::Error)]
enum ProcessError {
    #[error("Invalid input: {0}")]
    ParseError(#[from] std::num::ParseIntError),
    
    #[error("Value must be non-negative, got {0}")]
    NegativeValue(i32),
}

fn process(input: &str) -> Result<Output, ProcessError> {
    let parsed = input.parse::<i32>()?;
    if parsed < 0 {
        return Err(ProcessError::NegativeValue(parsed));
    }
    Ok(Output { value: parsed })
}
```

**TypeScript Example**:
```typescript
// ❌ Bad: Generic errors
throw new Error('User not found');

// ✅ Good: Typed errors
class UserNotFoundError extends Error {
  constructor(public userId: string) {
    super(`User not found: ${userId}`);
    this.name = 'UserNotFoundError';
  }
}

// Usage enables exhaustive handling
try {
  await getUser(id);
} catch (e) {
  if (e instanceof UserNotFoundError) {
    return res.status(404).json({ error: e.message });
  }
  if (e instanceof UnauthorizedError) {
    return res.status(401).json({ error: e.message });
  }
  throw e;  // Re-throw unknown errors
}
```

---

## Implementation Checklist

To implement architecture constraints:

1. **Document architectural decisions**:
   ```bash
   $ aeterna knowledge add adr \
       --id ADR-001 \
       --title "Strangler Facade for Legacy Migration" \
       --decision "All legacy integrations must use strangler facade"
   ```

2. **Create pattern examples**:
   ```bash
   $ mkdir -p docs/patterns
   $ # Create example implementations for each pattern
   ```

3. **Deploy as warnings**:
   ```bash
   $ aeterna policy import architecture-constraints.md \
       --scope org \
       --severity warn
   ```

4. **Train team on patterns**:
   - Schedule architecture review sessions
   - Create migration guides for existing code
   - Document exceptions process

5. **Gradually enforce**:
   ```bash
   # After 30 days
   $ aeterna policy update require-strangler-facade --severity error
   
   # After 60 days
   $ aeterna policy update require-strangler-facade --severity block
   ```

---

## Related Policies

- [Security Baseline](security-baseline.md) - Core security policies
- [Dependency Management](dependency-management.md) - Control packages
- [Code Quality](code-quality.md) - Maintain standards
- [Team Conventions](team-conventions.md) - Team-specific rules
