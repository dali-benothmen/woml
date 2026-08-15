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
  'NPM_TOKEN',
  '--provenance',
  'id-token: write',
  'prepare-main',
  'LICENSE',
]) {
  if (!workflow.includes(marker) && !releaseScript.includes(marker)) {
    throw new Error(`Native release boundary is missing ${marker}.`);
  }
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
  `WOML native release verified: ${womlNativeTargets.length} target packages, isolated woml-native build, runtime selection, license staging, provenance, and platform-first publication.`,
);
