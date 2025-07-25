# Contributing to Node-Cronflow

Thank you for your interest in contributing to Node-Cronflow! This document outlines the development workflow, contribution guidelines, and how to work with our versioning system.

## 🚀 Quick Start

1. **Fork** the repository
2. **Clone** your fork: `git clone https://github.com/YOUR_USERNAME/node-cronflow.git`
3. **Install dependencies**: `bun install`
4. **Build the project**: `bun run build:all`
5. **Run tests**: `bun test`
6. **Create a feature branch**: `git checkout -b feature/your-feature-name`

## 🛠️ Development Setup

### Prerequisites

- **Bun** >= 1.0.0 (recommended) or **Node.js** >= 18.0.0
- **Rust** >= 1.70.0 (for core development)
- **Git**

### Installation

```bash
# Clone the repository
git clone https://github.com/dali-benothmen/cronflow.git
cd node-cronflow

# Install dependencies
bun install

# Build everything (TypeScript + Rust core)
bun run build:all

# Run tests
bun test
```

### Available Scripts

```bash
# Development
bun run dev              # Start development server with watch mode
bun run dev:test         # Run tests in watch mode
bun run dev:build        # Build in watch mode

# Building
bun run build            # Build TypeScript only
bun run build:all        # Build TypeScript + Rust core
bun run build:complete   # Clean build with core binary copy
bun run build:prod       # Production build

# Testing
bun test                 # Run all tests
bun run test:run         # Run tests once
bun run test:coverage    # Run tests with coverage
bun run test:ui          # Run tests with UI

# Code Quality
bun run lint             # Run ESLint
bun run lint:fix         # Fix ESLint issues
bun run format           # Format code with Prettier
bun run format:check     # Check code formatting

# Package Management
bun run analyze          # Analyze bundle size
bun run check:bundle-size # Check bundle size limits
bun run clean            # Clean build artifacts
```

## 📝 Changesets Workflow

We use [Changesets](https://github.com/changesets/changesets) for versioning and publishing. This ensures consistent releases and automatic changelog generation.

### How Changesets Work

1. **Changeset Creation**: When making changes, create a changeset describing what changed
2. **Version Bumping**: Changesets automatically determine the next version based on change types
3. **Release Publishing**: Automated publishing to npm with proper changelog

### Creating a Changeset

When you make changes that should be included in a release:

```bash
# Create a changeset
bunx changeset
```

This will prompt you to:

1. **Select packages** that changed (if multiple)
2. **Choose change type**:
   - `patch` - Bug fixes, documentation updates
   - `minor` - New features (backward compatible)
   - `major` - Breaking changes
3. **Write a description** of your changes

### Changeset Types

```bash
# Patch release (0.1.0 → 0.1.1)
bunx changeset
# Select: patch
# Description: "fix: resolve webhook parsing issue"

# Minor release (0.1.0 → 0.2.0)
bunx changeset
# Select: minor
# Description: "feat: add parallel execution support"

# Major release (0.1.0 → 1.0.0)
bunx changeset
# Select: major
# Description: "BREAKING CHANGE: refactor API for better performance"
```

### Example Changeset File

After running `bunx changeset`, a file like `.changeset/blue-cats-smile.md` will be created:

```markdown
---
'node-cronflow': patch
---

fix: resolve webhook parsing issue in high-traffic scenarios

- Improved error handling for malformed webhook payloads
- Added validation for required webhook headers
- Enhanced logging for debugging webhook issues
```

### Version Management

```bash
# Version packages (creates/updates CHANGELOG.md)
bunx changeset version

# Publish to npm
bunx changeset publish
```

## 🔄 Pull Request Guidelines

### Before Submitting a PR

1. **Ensure tests pass**: `bun test`
2. **Check code quality**: `bun run lint && bun run format:check`
3. **Build successfully**: `bun run build:all`
4. **Create changeset** (if needed): `bunx changeset`

### PR Title Format

Use conventional commit format:

```
type(scope): description

Examples:
feat(sdk): add parallel execution support
fix(core): resolve memory leak in job dispatcher
docs(readme): update installation instructions
test(workflow): add integration tests for webhooks
```

### PR Description Template

```markdown
## Description

Brief description of what this PR accomplishes.

## Type of Change

- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update

## Testing

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing completed

## Checklist

- [ ] Code follows the style guidelines
- [ ] Self-review completed
- [ ] Changeset created (if needed)
- [ ] Documentation updated (if needed)

## Related Issues

Closes #123
```

### Code Style Guidelines

#### TypeScript/JavaScript

- **Indentation**: 2 spaces
- **Quotes**: Single quotes for strings
- **Semicolons**: Always use semicolons
- **Trailing commas**: Use trailing commas in objects and arrays
- **Line length**: 80 characters max

```typescript
// Good
const config = {
  name: 'node-cronflow',
  version: '0.1.0',
  description: 'Workflow automation engine',
};

// Bad
const config = {
  name: 'node-cronflow',
  version: '0.1.0',
  description: 'Workflow automation engine',
};
```

#### Rust

- **Indentation**: 4 spaces
- **Line length**: 100 characters max
- **Naming**: snake_case for variables and functions
- **Documentation**: Include doc comments for public APIs

```rust
/// Executes a workflow step with the given context
pub fn execute_step(&self, step_id: &str, context: &Context) -> CoreResult<StepResult> {
    // Implementation
}
```

### Commit Message Guidelines

Follow [Conventional Commits](https://www.conventionalcommits.org/) format:

```
type(scope): description

[optional body]

[optional footer]
```

#### Types

- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

#### Examples

```bash
# Good commit messages
feat(sdk): add parallel execution support
fix(core): resolve memory leak in job dispatcher
docs(readme): update installation instructions
test(workflow): add integration tests for webhooks

# Bad commit messages
added stuff
fixed bug
updated docs
```

## 🧪 Testing Guidelines

### Writing Tests

- **Unit tests**: Test individual functions and classes
- **Integration tests**: Test component interactions
- **End-to-end tests**: Test complete workflows

### Test Structure

```typescript
// Unit test example
describe('WorkflowBuilder', () => {
  describe('step()', () => {
    it('should add a step to the workflow', () => {
      const workflow = new WorkflowBuilder();
      workflow.step('test-step', async ctx => ({ result: 'success' }));

      expect(workflow.steps).toHaveLength(1);
      expect(workflow.steps[0].id).toBe('test-step');
    });
  });
});
```

### Running Tests

```bash
# Run all tests
bun test

# Run tests in watch mode
bun run dev:test

# Run tests with coverage
bun run test:coverage

# Run specific test file
bun test tests/workflow.test.ts
```

## 📚 Documentation Guidelines

### Code Documentation

- **Public APIs**: Always include JSDoc comments
- **Complex logic**: Add inline comments explaining the reasoning
- **Examples**: Include usage examples in documentation

````typescript
/**
 * Creates a new workflow instance
 * @param name - The name of the workflow
 * @param options - Configuration options
 * @returns A new workflow builder instance
 * @example
 * ```typescript
 * const workflow = new WorkflowBuilder('my-workflow', {
 *   timeout: 30000,
 *   retries: 3
 * });
 * ```
 */
export class WorkflowBuilder {
  constructor(name: string, options?: WorkflowOptions) {
    // Implementation
  }
}
````

### README Updates

- Update README.md when adding new features
- Include usage examples
- Update installation instructions if needed
- Add troubleshooting section for common issues

## 🔧 Development Workflow

### Feature Development

1. **Create feature branch**: `git checkout -b feature/your-feature`
2. **Make changes**: Implement your feature
3. **Add tests**: Write tests for your feature
4. **Run tests**: `bun test`
5. **Check code quality**: `bun run lint && bun run format:check`
6. **Create changeset**: `bunx changeset` (if needed)
7. **Commit changes**: Use conventional commit format
8. **Push branch**: `git push origin feature/your-feature`
9. **Create PR**: Submit pull request

### Bug Fixes

1. **Create fix branch**: `git checkout -b fix/issue-description`
2. **Reproduce issue**: Create test case that reproduces the bug
3. **Fix the issue**: Implement the fix
4. **Add regression test**: Ensure the bug doesn't return
5. **Test thoroughly**: `bun test`
6. **Create changeset**: `bunx changeset`
7. **Submit PR**: Follow PR guidelines

### Release Process

1. **Merge PRs**: Merge approved pull requests to main
2. **Version packages**: `bunx changeset version`
3. **Review changelog**: Check generated CHANGELOG.md
4. **Publish**: `bunx changeset publish` (automated in CI)

## 🐛 Reporting Issues

### Bug Reports

When reporting bugs, please include:

1. **Environment**: OS, Node.js/Bun version, package version
2. **Steps to reproduce**: Clear, step-by-step instructions
3. **Expected behavior**: What should happen
4. **Actual behavior**: What actually happens
5. **Error messages**: Full error stack traces
6. **Code example**: Minimal code that reproduces the issue

### Feature Requests

When requesting features, please include:

1. **Use case**: Why this feature is needed
2. **Proposed solution**: How you envision it working
3. **Alternatives considered**: Other approaches you've thought about
4. **Impact**: How this affects existing functionality

## 🤝 Getting Help

- **GitHub Issues**: For bug reports and feature requests
- **GitHub Discussions**: For questions and general discussion
- **Documentation**: Check the README and inline code documentation

## 📄 License

By contributing to Node-Cronflow, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to Node-Cronflow! 🚀
