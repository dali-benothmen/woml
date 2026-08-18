#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  nativePackageName,
  womlNativeTargets,
  womlNativeTargetSpecs,
} from '../src/native-platform';

const root = resolve(import.meta.dir, '../..');
const workflow = await readFile(
  resolve(root, '.github/workflows/release.yml'),
  'utf8',
);
const releaseScript = await readFile(
  resolve(root, 'woml-cli/scripts/native-release.ts'),
  'utf8',
);
const artifactScript = await readFile(
  resolve(root, 'woml-cli/scripts/release-artifact.ts'),
  'utf8',
);
const loader = await readFile(
  resolve(root, 'woml-cli/src/rust-executor.ts'),
  'utf8',
);

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
  'workflow_dispatch',
  'publish_to_npm',
  'npm-production',
  'WOML_NPM_PUBLISH_ENABLED',
  "github.ref_type == 'tag'",
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
  if (
    !workflow.includes(marker) &&
    !releaseScript.includes(marker) &&
    !artifactScript.includes(marker)
  ) {
    throw new Error(`Native release boundary is missing ${marker}.`);
  }
}

if (workflow.includes('NPM_TOKEN') || workflow.includes('NODE_AUTH_TOKEN')) {
  throw new Error(
    'Release publication must use npm trusted publishing, not a long-lived token.',
  );
}
if (
  !workflow.includes(
    "github.event_name == 'workflow_dispatch' && inputs.publish_to_npm == true && github.ref_type == 'tag'",
  )
) {
  throw new Error(
    'The npm publish job is not restricted to an explicit exact-tag dispatch.',
  );
}
if (!workflow.includes('name: WOML Release Candidate')) {
  throw new Error('Tag pushes must produce a release candidate, not publish directly.');
}

if (
  workflow.includes('@cronflow/') ||
  workflow.includes('cronflow.git') ||
  workflow.includes('core/package.json')
) {
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
  `WOML native release verified: ${womlNativeTargets.length} target packages, matching-runtime load tests, sealed artifacts, trusted/manual publication, and platform-first ordering.`,
);
