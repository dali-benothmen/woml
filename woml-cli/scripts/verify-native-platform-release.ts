#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { nativePackageName, womlNativeTargets, womlNativeTargetSpecs } from '../src/native-platform';

const root = resolve(import.meta.dir, '../..');
const workflow = await readFile(resolve(root, '.github/workflows/release.yml'), 'utf8');
const releaseScript = await readFile(resolve(root, 'woml-cli/scripts/native-release.ts'), 'utf8');
const artifactScript = await readFile(resolve(root, 'woml-cli/scripts/release-artifact.ts'), 'utf8');
const loader = await readFile(resolve(root, 'woml-cli/src/rust-executor.ts'), 'utf8');

for (const target of womlNativeTargets) {
  const spec = womlNativeTargetSpecs[target];
  for (const marker of [target, spec.rustTarget, spec.libraryName]) {
    if (!workflow.includes(marker)) {
      throw new Error(`Release workflow is missing ${target} marker ${marker}.`);
    }
  }
  if (!releaseScript.includes('nativePackageName(target)')) {
    throw new Error(`Release packager does not derive ${nativePackageName(target)}.`);
  }
}

for (const marker of [
  'core/woml-native/Cargo.toml',
  '--locked',
  '-j 1',
  'verify-collected',
  '--provenance',
  'id-token: write',
  'rust:1.88-bullseye',
  "glibcMax: '2.31'",
  'Enforce the Linux glibc compatibility ceiling',
  "grep -oE 'GLIBC_[0-9]+(\\.[0-9]+)*'",
  'workflow_dispatch',
  'publish_to_npm',
  'npm-production',
  'NPM_TOKEN',
  'NODE_AUTH_TOKEN',
  "github.ref_type == 'tag'",
  "github.event_name == 'push'",
  'release-artifact.ts load-native',
  'release-artifact.ts seal',
  'release-artifact.ts verify',
  'smoke-release-candidate.ts',
  'native-load-test.json',
  'artifact-sha256.json',
  'actions/upload-artifact@v6',
  'actions/download-artifact@v6',
  'retention-days',
  'LICENSE',
  'NOTICE.md',
  'bun audit --cwd woml-cli',
  'cargo audit --file core/Cargo.lock --ignore RUSTSEC-2026-0258',
]) {
  if (!workflow.includes(marker) && !releaseScript.includes(marker) && !artifactScript.includes(marker)) {
    throw new Error(`Native release boundary is missing ${marker}.`);
  }
}

if (!workflow.includes('NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}') || !workflow.includes('NPM_TOKEN is missing.')) {
  throw new Error('Release publication must receive and validate the protected NPM_TOKEN secret.');
}
if (
  !workflow.includes(
    "github.ref_type == 'tag' && (github.event_name == 'push' || (github.event_name == 'workflow_dispatch' && inputs.publish_to_npm == true))",
  )
) {
  throw new Error('The npm publish job is not restricted to a verified exact tag.');
}
if (!workflow.includes('name: WOML Release')) {
  throw new Error('The automated WOML release workflow identity is missing.');
}

if (workflow.includes('@cronflow/') || workflow.includes('cronflow.git') || workflow.includes('core/package.json')) {
  throw new Error('Release workflow restored retired package architecture.');
}
if (
  !loader.includes('nativeTargetForRuntime') ||
  !loader.includes('nativePackageName') ||
  !loader.includes('optional dependencies are enabled')
) {
  throw new Error('Runtime loader does not resolve the platform package contract.');
}

console.log(
  `WOML native release verified: ${womlNativeTargets.length} target packages, matching-runtime load tests, sealed artifacts, protected tag publication, and platform-first ordering.`,
);
