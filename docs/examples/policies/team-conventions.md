# Team Conventions Policies

Team-specific coding conventions and standards.

---

## 1. Naming Convention: Snake Case (APIs)

**Natural Language**: "Use snake_case for all API field names and endpoints"

**Cedar Policy**:
```cedar
// Policy: naming-convention-snake-api
// Scope: team (API Team)
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*/(api|routes|endpoints)/.*\\.(ts|js)$") &&
  resource.json_fields.any(field =>
    !field.name.matches("^[a-z][a-z0-9_]*$")
  )
}
advice {
  message: "API field '{field.name}' should use snake_case. Example: user_id, created_at"
};

// Check OpenAPI specs
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*openapi.*\\.(yaml|json)$") &&
  resource.schema_properties.any(prop =>
    !prop.name.matches("^[a-z][a-z0-9_]*$")
  )
};
```

**Explanation**:
Snake_case is the standard for REST APIs, especially when integrating with Python, Ruby, or database systems that use snake_case. Consistency aids API consumers.

**Use Cases**:
- API consistency
- Cross-language compatibility
- Database alignment

**Examples**:
```json
// ✅ Good: snake_case
{
  "user_id": "usr_123",
  "created_at": "2024-01-15T10:30:00Z",
  "is_active": true,
  "email_address": "user@example.com"
}

// ❌ Bad: camelCase in API
{
  "userId": "usr_123",
  "createdAt": "2024-01-15T10:30:00Z",
  "isActive": true,
  "emailAddress": "user@example.com"
}
```

**Serialization Setup**:
```typescript
// Use class-transformer with snake_case strategy
import { classToPlain, plainToClass } from 'class-transformer';
import { snakeCase } from 'lodash-es';

const serializerOptions = {
  strategy: 'excludeAll',
  transformFn: (key: string) => snakeCase(key)
};
```

---

## 2. Naming Convention: Camel Case (Variables)

**Natural Language**: "Use camelCase for all JavaScript/TypeScript variables and function names"

**Cedar Policy**:
```cedar
// Policy: naming-convention-camel-vars
// Scope: team
// Severity: warn

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.path.matches(".*\\.(ts|js|tsx|jsx)$") &&
  resource.variables.any(v =>
    v.scope == "local" &&
    !v.name.matches("^[a-z][a-zA-Z0-9]*$") &&
    !v.name.matches("^_[a-z][a-zA-Z0-9]*$")  // Allow _private
  )
}
advice {
  message: "Variable '{v.name}' should use camelCase. Example: userId, isActive"
};

// Constants should be SCREAMING_SNAKE_CASE
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.constants.any(c =>
    !c.name.matches("^[A-Z][A-Z0-9_]*$")
  )
}
advice {
  message: "Constant '{c.name}' should use SCREAMING_SNAKE_CASE. Example: MAX_RETRIES"
};
```

**Explanation**:
CamelCase is the JavaScript/TypeScript standard. Following language conventions makes code more readable and matches library usage.

**Use Cases**:
- Code consistency
- Readability
- Industry standards

**Naming Patterns**:
| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `userName`, `isActive` |
| Functions | camelCase | `getUserById`, `calculateTotal` |
| Classes | PascalCase | `UserService`, `PaymentProcessor` |
| Interfaces | PascalCase | `UserRepository`, `PaymentGateway` |
| Constants | SCREAMING_SNAKE | `MAX_RETRIES`, `API_BASE_URL` |
| Enums | PascalCase/SCREAMING | `UserRole`, `HTTP_STATUS` |
| Files | kebab-case | `user-service.ts`, `payment-utils.ts` |

---

## 3. Require CHANGELOG

**Natural Language**: "All projects must maintain a CHANGELOG.md following Keep a Changelog format"

**Cedar Policy**:
```cedar
// Policy: require-changelog
// Scope: org
// Severity: warn

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  !resource.files.contains("CHANGELOG.md")
}
advice {
  message: "Add CHANGELOG.md to document changes. See: https://keepachangelog.com"
};

// Require changelog update with version bumps
forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  resource.changed_files.any(f => 
    f.path.matches("package\\.json|Cargo\\.toml") &&
    f.changes.any(c => c.field == "version")
  ) &&
  !resource.changed_files.any(f => f.path == "CHANGELOG.md")
}
advice {
  message: "Version bump detected but CHANGELOG.md not updated"
};
```

**Explanation**:
A changelog provides a human-readable history of notable changes. It helps users understand what changed between versions and aids debugging.

**Use Cases**:
- Release communication
- Debugging regressions
- Compliance documentation

**CHANGELOG Template**:
```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New feature description

### Changed
- Changes in existing functionality

### Deprecated
- Soon-to-be removed features

### Removed
- Removed features

### Fixed
- Bug fixes

### Security
- Vulnerability fixes

## [1.0.0] - 2024-01-15

### Added
- Initial release
- User authentication system
- API rate limiting
```

---

## 4. Require README

**Natural Language**: "Every project must have a README.md with standard sections"

**Cedar Policy**:
```cedar
// Policy: require-readme
// Scope: tenant
// Severity: error

forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  !resource.files.contains("README.md")
}
advice {
  message: "Add README.md with project documentation"
};

// Require specific sections
forbid(
  principal,
  action == Action::"Deploy",
  resource
)
when {
  resource.files.contains("README.md") &&
  !resource.readme.sections.containsAll([
    "description",
    "installation", 
    "usage"
  ])
}
advice {
  message: "README.md missing required sections: description, installation, usage"
};
```

**Explanation**:
A README is the first thing people see. It should explain what the project does, how to set it up, and how to use it.

**Use Cases**:
- Onboarding
- Documentation
- Open source compliance

**README Template**:
````markdown
# Project Name

Brief description of what this project does.

## Table of Contents

- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [API Reference](#api-reference)
- [Contributing](#contributing)
- [License](#license)

## Installation

```bash
npm install project-name
```

## Usage

```typescript
import { Feature } from 'project-name';

const result = await Feature.doSomething();
```

## Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `API_KEY` | API authentication key | required |
| `DEBUG` | Enable debug logging | `false` |

## API Reference

See [API Documentation](./docs/api.md).

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

[MIT](./LICENSE)
````

---

## 5. Conventional Commits

**Natural Language**: "All commit messages must follow Conventional Commits format"

**Cedar Policy**:
```cedar
// Policy: conventional-commits
// Scope: org
// Severity: error

forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  !resource.message.matches("^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\\([a-z-]+\\))?!?:\\s.+")
}
advice {
  message: "Commit message must follow Conventional Commits: type(scope): description"
};

// Require scope for certain types
forbid(
  principal,
  action == Action::"Commit",
  resource
)
when {
  resource.message.matches("^(feat|fix):") &&
  !resource.message.matches("^(feat|fix)\\([a-z-]+\\):")
}
advice {
  message: "feat and fix commits should include scope: feat(auth): add login"
};
```

**Explanation**:
Conventional Commits enable automated changelog generation, semantic versioning, and clear commit history. The format is: `type(scope): description`.

**Use Cases**:
- Automated changelogs
- Semantic versioning
- Clear git history

**Commit Types**:
| Type | Description | Bumps |
|------|-------------|-------|
| `feat` | New feature | Minor |
| `fix` | Bug fix | Patch |
| `docs` | Documentation only | - |
| `style` | Formatting, missing semicolons | - |
| `refactor` | Code change that neither fixes nor adds | - |
| `perf` | Performance improvement | Patch |
| `test` | Adding tests | - |
| `build` | Build system, dependencies | - |
| `ci` | CI configuration | - |
| `chore` | Other changes | - |

**Examples**:
```bash
# ✅ Good
feat(auth): add OAuth2 login support
fix(api): handle null response from payment provider
docs(readme): add installation instructions
refactor(user): extract validation to separate module
perf(query): add index for user lookups
test(auth): add integration tests for login flow

# Breaking changes (note the !)
feat(api)!: change response format to JSON:API

# ❌ Bad
fixed bug
update code
WIP
```

**Tooling**:
```bash
# Install commitlint
npm install -D @commitlint/cli @commitlint/config-conventional

# commitlint.config.js
module.exports = { extends: ['@commitlint/config-conventional'] };

# Add husky hook
npx husky add .husky/commit-msg 'npx --no -- commitlint --edit "$1"'
```

---

## 6. Require PR Template

**Natural Language**: "All pull requests must use the standard PR template"

**Cedar Policy**:
```cedar
// Policy: require-pr-template
// Scope: org
// Severity: warn

forbid(
  principal,
  action == Action::"CreatePullRequest",
  resource
)
when {
  !resource.repository.files.contains(".github/PULL_REQUEST_TEMPLATE.md")
}
advice {
  message: "Add .github/PULL_REQUEST_TEMPLATE.md to standardize PR descriptions"
};

// Require template sections to be filled
forbid(
  principal,
  action == Action::"Merge",
  resource
)
when {
  resource.type == "pull_request" &&
  (
    resource.body.matches("\\[\\s*\\]\\s*I have tested") ||  // Unchecked box
    resource.body.matches("<!-- .* -->") ||                   // Unfilled placeholder
    resource.body.length < 50                                 // Too short
  )
}
advice {
  message: "Please complete all sections of the PR template"
};
```

**Explanation**:
PR templates ensure consistent information is provided, making reviews faster and more effective. They also serve as a checklist for contributors.

**Use Cases**:
- Review efficiency
- Quality assurance
- Knowledge transfer

**PR Template**:
```markdown
<!-- .github/PULL_REQUEST_TEMPLATE.md -->

## Summary

Brief description of changes.

## Type of Change

- [ ] Bug fix (non-breaking change fixing an issue)
- [ ] New feature (non-breaking change adding functionality)
- [ ] Breaking change (fix or feature causing existing functionality to break)
- [ ] Documentation update
- [ ] Refactoring (no functional changes)

## Related Issues

Fixes #(issue number)

## Changes Made

- Change 1
- Change 2

## Testing

- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] Manual testing performed

### Test Instructions

1. Step to reproduce/test

## Screenshots (if applicable)

## Checklist

- [ ] My code follows the project's style guidelines
- [ ] I have performed a self-review
- [ ] I have commented hard-to-understand areas
- [ ] I have updated documentation
- [ ] My changes generate no new warnings
- [ ] New and existing tests pass
```

---

## Implementation Checklist

To implement team conventions:

1. **Document conventions**:
   ```bash
   $ mkdir -p docs/conventions
   $ # Create convention documentation
   ```

2. **Configure linting**:
   ```json
   // .eslintrc.json
   {
     "rules": {
       "camelcase": ["error", { "properties": "never" }],
       "@typescript-eslint/naming-convention": [
         "error",
         { "selector": "variable", "format": ["camelCase", "UPPER_CASE"] },
         { "selector": "function", "format": ["camelCase"] },
         { "selector": "typeLike", "format": ["PascalCase"] }
       ]
     }
   }
   ```

3. **Set up commit hooks**:
   ```bash
   $ npm install -D husky @commitlint/cli @commitlint/config-conventional
   $ npx husky install
   $ npx husky add .husky/commit-msg 'npx --no -- commitlint --edit "$1"'
   ```

4. **Add templates**:
   ```bash
   $ mkdir -p .github
   $ touch .github/PULL_REQUEST_TEMPLATE.md
   $ touch .github/ISSUE_TEMPLATE/bug_report.md
   $ touch .github/ISSUE_TEMPLATE/feature_request.md
   ```

5. **Import policies**:
   ```bash
   $ aeterna policy import team-conventions.md \
       --scope team \
       --mode enforce
   ```

---

## Team Onboarding

When new team members join:

1. **Share convention docs**:
   ```bash
   $ aeterna knowledge show conventions --scope team
   ```

2. **Run policy check on their first PR**:
   ```bash
   $ aeterna policy check --pr 123 --verbose
   ```

3. **Provide feedback with fix suggestions**:
   ```bash
   $ aeterna govern suggest --violations pr-123
   ```

---

## Customization by Team

Different teams may have different conventions:

```cedar
// Backend team: snake_case for Python services
permit(
  principal,
  action == Action::"Commit",
  resource
)
when {
  principal.team == "backend" &&
  resource.path.matches(".*\\.py$") &&
  resource.variables.all(v => v.name.matches("^[a-z][a-z0-9_]*$"))
};

// Frontend team: camelCase for TypeScript
permit(
  principal,
  action == Action::"Commit", 
  resource
)
when {
  principal.team == "frontend" &&
  resource.path.matches(".*\\.tsx?$") &&
  resource.variables.all(v => v.name.matches("^[a-z][a-zA-Z0-9]*$"))
};
```

---

## Related Policies

- [Security Baseline](security-baseline.md) - Core security policies
- [Dependency Management](dependency-management.md) - Control packages
- [Architecture Constraints](architecture-constraints.md) - Enforce patterns
- [Code Quality](code-quality.md) - Maintain standards
