# Release Workflow Fixes

## Issues Fixed

### 1. Node.js Version Mismatch

**Problem:** Workflow used Node.js 18, but dependencies require Node.js 20+

- `semantic-release` requires Node >= 20.8.1
- `@octokit/*` packages require Node >= 20

**Fix:**

- Updated `.github/workflows/release.yml` to use Node 20
- Updated `package.json` engines to require Node >= 20
- Updated `core/package.json` engines to require Node >= 20

### 2. Platform-Specific Dependencies

**Problem:** `@rollup/rollup-linux-x64-gnu` was being installed on macOS/Windows causing platform mismatch errors

**Fix:**

- Added `--omit=optional` flag when installing main package dependencies
- This prevents platform-specific optional dependencies from causing issues

### 3. NAPI Command Not Found in Docker

**Problem:** Docker containers didn't have the `napi` command available

**Fix:**

- Changed build commands to install dependencies first: `npm install`
- Use `npx napi` instead of just `napi` to use the local installation
- Moved `@napi-rs/cli` from devDependencies to dependencies in `core/package.json`

### 4. Missing Package.json Reference

**Problem:** Build commands referenced `--package-json-path ../package.json` incorrectly

**Fix:**

- Removed the `--package-json-path` flag
- NAPI-RS now uses the `package.json` in the `core/` directory directly

### 5. Build Process Improvements

**Fix:**

- Added `set -e` to docker build scripts to fail fast on errors
- Added `npm install` step before building in each platform
- Fixed path navigation in publish step

## Changes Made

### Files Modified:

1. **`.github/workflows/release.yml`**
   - Updated Node version: 18 → 20
   - Fixed build commands to install dependencies
   - Changed `napi` to `npx napi`
   - Removed incorrect `--package-json-path` flag
   - Added `--omit=optional` to main package install
   - Fixed publish loop path navigation

2. **`package.json`**
   - Updated engines: `node: ">=20.0.0"`

3. **`core/package.json`**
   - Updated engines: `node: ">= 20"`
   - Moved `@napi-rs/cli` from devDependencies to dependencies
   - Added `prepublish:artifacts` script

## Next Steps to Release

### 1. Commit and Push Fixes

```bash
git add .github/workflows/release.yml package.json core/package.json
git commit -m "fix: update release workflow for cross-platform builds"
git push origin master
```

### 2. Delete Old Tag (if it exists)

```bash
# Delete local tag
git tag -d v0.10.0

# Delete remote tag
git push origin :refs/tags/v0.10.0
```

### 3. Create New Release

```bash
# Ensure versions are updated
npm version patch  # or minor/major

# Update core version to match
cd core
npm version patch
cd ..

# Update optionalDependencies versions in package.json manually

# Commit version bumps
git add package.json core/package.json package-lock.json
git commit -m "chore: bump version to 0.10.1"
git push origin master

# Create and push new tag
git tag v0.10.1
git push origin v0.10.1
```

### 4. Monitor Release

- Go to: https://github.com/dali-benothmen/cronflow/actions
- Watch the "Release" workflow
- All 7 platform builds should now succeed
- Platform packages will be published to `@cronflow/*`
- Main package will be published as `cronflow`

## Expected Workflow Stages

1. ✅ **Build Stages** (parallel, ~10-15 min each):
   - Build x86_64-apple-darwin
   - Build aarch64-apple-darwin
   - Build x86_64-pc-windows-msvc
   - Build aarch64-pc-windows-msvc
   - Build x86_64-unknown-linux-gnu
   - Build x86_64-unknown-linux-musl
   - Build aarch64-unknown-linux-gnu

2. ✅ **Publish Stage** (~5 min):
   - Download all artifacts
   - Generate platform packages
   - Publish 7 platform packages to `@cronflow/*`
   - Build main package
   - Publish main package to `cronflow`

## Testing After Release

```bash
# Test on Windows
npm install cronflow
npm list @cronflow/win32-x64-msvc

# Test workflow execution
node -e "const {cronflow} = require('cronflow'); cronflow.start().then(r => console.log('Success:', r))"
```

Should no longer return `undefined`! ✅

## Troubleshooting

If builds still fail:

1. **Check Node version in logs**: Should show v20.x.x
2. **Check napi installation**: Should show `npm install` running in core/
3. **Check artifact upload**: Should find `*.node` files
4. **Check npm token**: Verify `NPM_TOKEN` secret is set correctly

## Key Improvements

- ✅ Uses correct Node.js version (20)
- ✅ Installs dependencies in Docker containers
- ✅ Uses npx to run locally-installed napi
- ✅ Avoids platform-specific dependency conflicts
- ✅ Properly navigates paths during publishing
- ✅ Fails fast with `set -e` in bash scripts
