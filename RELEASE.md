# Release Process

This document explains how Cronflow's cross-platform release process works.

## Overview

Cronflow uses GitHub Actions to automatically build native binaries for all supported platforms and publish them to npm. This ensures Windows, macOS, and Linux users all get optimized, pre-compiled binaries without needing to compile from source.

## How It Works

### 1. Platform-Specific Packages

When you install `cronflow`, npm automatically installs the correct platform-specific package as an optional dependency:

- **Windows x64**: `@cronflow/win32-x64-msvc`
- **Windows ARM64**: `@cronflow/win32-arm64-msvc`
- **macOS Intel**: `@cronflow/darwin-x64`
- **macOS Apple Silicon**: `@cronflow/darwin-arm64`
- **Linux x64 (GNU)**: `@cronflow/linux-x64-gnu`
- **Linux x64 (musl)**: `@cronflow/linux-x64-musl`
- **Linux ARM64 (GNU)**: `@cronflow/linux-arm64-gnu`

### 2. Automated Release Workflow

The release process is fully automated via GitHub Actions (`.github/workflows/release.yml`):

1. **Trigger**: Push a git tag matching `v*` (e.g., `v0.9.1`)
2. **Build**: GitHub Actions builds native binaries on multiple platforms in parallel
3. **Publish**: Platform-specific packages are published first, then the main package

## Making a Release

### Prerequisites

1. Ensure you have npm publish access to the `@cronflow` organization
2. Set up the `NPM_TOKEN` secret in GitHub repository settings
3. Ensure all tests pass: `npm test`

### Step-by-Step Release

1. **Update version in package.json files:**

   ```bash
   # Update main package.json
   npm version patch  # or minor, major

   # Update core/package.json to match
   cd core
   npm version patch
   cd ..

   # Commit the version changes
   git add package.json core/package.json
   git commit -m "chore: bump version to 0.9.1"
   ```

2. **Create and push a git tag:**

   ```bash
   git tag v0.9.1
   git push origin master
   git push origin v0.9.1
   ```

3. **Monitor the release:**
   - Go to GitHub Actions: `https://github.com/dali-benothmen/cronflow/actions`
   - Watch the "Release" workflow execute
   - Builds typically take 20-30 minutes (parallel builds across platforms)

4. **Verify the release:**

   ```bash
   # Check main package
   npm view cronflow version

   # Check platform packages
   npm view @cronflow/win32-x64-msvc version
   npm view @cronflow/darwin-arm64 version
   npm view @cronflow/linux-x64-gnu version
   ```

### Release Workflow Details

The GitHub Actions workflow (`.github/workflows/release.yml`) performs these steps:

#### Build Job (runs in parallel for each platform)

1. Checkout code
2. Set up Node.js and Rust toolchain
3. Install dependencies
4. Build native binary for the target platform
5. Upload artifact

#### Publish Job (runs after all builds complete)

1. Download all platform artifacts
2. Generate platform-specific packages using NAPI-RS
3. Publish each platform package to npm
4. Build and publish the main `cronflow` package

## Troubleshooting

### Build Failures

If a platform build fails:

1. Check the GitHub Actions logs for that platform
2. Common issues:
   - Rust compilation errors (check `core/src/`)
   - Missing dependencies
   - Incorrect target configuration

### Publish Failures

If npm publish fails:

1. Verify `NPM_TOKEN` secret is correctly set
2. Ensure you have publish access to `@cronflow` organization
3. Check if the version already exists on npm (can't republish same version)

### Missing Platform Package

If users report a platform package is missing:

1. Check if that platform's build succeeded in GitHub Actions
2. Verify the platform package was published to npm
3. Users can manually install: `npm install @cronflow/[platform-name]`

## Manual Release (Emergency)

If the automated workflow fails, you can release manually:

### Build All Platforms Locally (requires Docker)

```bash
# Build for all platforms using Docker
cd core
npm run build:core:all
```

### Publish Platform Packages

```bash
cd core
npm run prepublish:artifacts
cd npm/@cronflow/win32-x64-msvc && npm publish --access public && cd -
cd npm/@cronflow/darwin-arm64 && npm publish --access public && cd -
# ... repeat for all platforms
```

### Publish Main Package

```bash
npm run build:prod
npm publish --access public
```

## Version Management

We follow [Semantic Versioning](https://semver.org/):

- **MAJOR** (1.0.0): Breaking changes
- **MINOR** (0.9.0): New features, backwards compatible
- **PATCH** (0.9.1): Bug fixes, backwards compatible

All package versions (main + platform packages) should match.

## Testing a Release

Before making an official release, test the workflow:

1. **Create a test tag:**

   ```bash
   git tag v0.9.1-test
   git push origin v0.9.1-test
   ```

2. **Monitor the workflow** to ensure all builds succeed

3. **Delete the test packages** from npm (can't delete after 72 hours)

4. **Delete the test tag:**
   ```bash
   git tag -d v0.9.1-test
   git push origin :refs/tags/v0.9.1-test
   ```

## Post-Release

After a successful release:

1. **Update CHANGELOG.md** with release notes
2. **Create a GitHub Release** with notes and binaries
3. **Announce** on relevant channels (Discord, Twitter, etc.)
4. **Monitor** npm downloads and issue reports

## CI/CD Pipeline

The complete CI/CD pipeline includes:

1. **CI** (`.github/workflows/ci.yml`): Runs on every push
   - Runs tests
   - Builds for Linux (fastest for CI)
   - Uses Changesets for version management

2. **Release** (`.github/workflows/release.yml`): Runs on git tags
   - Builds for all platforms
   - Publishes to npm

## Additional Resources

- [NAPI-RS Documentation](https://napi.rs/)
- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [npm Publishing Documentation](https://docs.npmjs.com/cli/v8/commands/npm-publish)
