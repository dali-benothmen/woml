#!/usr/bin/env bun

import { stat, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createRequire } from 'node:module';

import { packagedAddonName } from './stage-native';

const root = resolve(import.meta.dir, '../..');
const packageRoot = resolve(root, 'woml-cli');

const packageJson = JSON.parse(
  await readFile(resolve(packageRoot, 'package.json'), 'utf8')
) as { scripts?: Record<string, string> };
const nativeBuild = packageJson.scripts?.['build:native'] ?? '';
if (
  !nativeBuild.includes('../core/woml-native/Cargo.toml') ||
  !nativeBuild.includes('--locked') ||
  nativeBuild.includes('--manifest-path ../core/Cargo.toml')
) {
  throw new Error(
    'The CLI native build is not pinned to the dedicated woml-native crate.'
  );
}
if (
  !packageJson.scripts?.['test:release']?.includes(
    'test:native-release-boundary'
  )
) {
  throw new Error(
    'The repository release journey does not include the WOML native boundary gate.'
  );
}

const nativeManifest = await readFile(
  resolve(root, 'core/woml-native/Cargo.toml'),
  'utf8'
);
const localDependencies = nativeManifest
  .split(/\r?\n/u)
  .filter(line => line.includes('path ='));
if (
  localDependencies.length !== 1 ||
  !localDependencies[0]?.includes('woml-engine')
) {
  throw new Error(
    `woml-native acquired an unexpected local dependency: ${localDependencies.join(', ')}`
  );
}

const workspaceManifest = await readFile(
  resolve(root, 'core/Cargo.toml'),
  'utf8'
);
if (
  workspaceManifest.includes('[package]') ||
  !workspaceManifest.includes('members = ["woml-engine", "woml-native"]')
) {
  throw new Error('The core workspace is not WOML-only.');
}
for (const retiredPath of [
  'core/src/lib.rs',
  'core/src/bridge.rs',
  'core/src/schema.sql',
  'core/build.rs',
  'core/package.json',
]) {
  if (await Bun.file(resolve(root, retiredPath)).exists()) {
    throw new Error(`The retired legacy Rust package returned at ${retiredPath}.`);
  }
}

const artifact = resolve(
  packageRoot,
  'dist',
  packagedAddonName(process.platform, process.arch)
);
if ((await stat(artifact)).size === 0) {
  throw new Error('The staged WOML native addon is empty.');
}
const require = createRequire(import.meta.url);
const exports = Object.keys(require(artifact) as Record<string, unknown>);
for (const legacyExport of [
  'registerWorkflow',
  'createRun',
  'executeStep',
  'startWebhookServer',
]) {
  if (exports.includes(legacyExport)) {
    throw new Error(
      `The packaged WOML addon still exports legacy symbol ${legacyExport}.`
    );
  }
}
if (exports.length !== 36 || !exports.includes('executeWomlWorkflow')) {
  throw new Error(
    `The packaged WOML addon has an unexpected export surface (${exports.length} exports).`
  );
}

console.log(
  `WOML native release boundary verified: WOML-only Cargo workspace, dedicated locked build, stable ${packagedAddonName(
    process.platform,
    process.arch
  )} artifact, 36 WOML-only exports, development override coverage, and clean-package release wiring.`
);
