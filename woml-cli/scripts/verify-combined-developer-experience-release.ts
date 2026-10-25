#!/usr/bin/env bun

import { readdir, readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  packedPackageFiles,
  publicProjectFiles,
} from './release-contract';
import { expectedNativeExports } from './verify-native-exports';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> => readFile(resolve(root, path), 'utf8');
const required = [
  'docs/project-manifest.md',
  'docs/migrating-to-projects.md',
  'docs/infrastructure-agent-tutorial.md',
  'docs/combined-developer-experience-troubleshooting.md',
  'docs/combined-developer-experience-operator-runbook.md',
  'docs/protocols/combined-developer-experience-v1.md',
  'docs/schemas/woml-project.v1.schema.json',
  'docs/schemas/project-check.v1.schema.json',
  'woml-cli/templates/woml.project.json',
  'woml-cli/templates/AGENTS.md',
  'woml-cli/templates/workflows/hello.woml',
  'woml-cli/tests/combined-developer-experience.test.ts',
  'woml-cli/tests/combined-developer-experience-packaged-release.test.ts',
  'woml-cli/scripts/benchmark-combined-developer-experience.ts',
];
for (const path of required) {
  if ((await stat(resolve(root, path))).size === 0) {
    throw new Error(`CDX8 release artifact is missing or empty: ${path}`);
  }
}

const budgets = JSON.parse(await read(
  'woml/tests/fixtures/combined-developer-experience/budgets.v1.json'
));
if (
  budgets.baseline.commit !== '681ad9a80009d53bfe8853852a2443f9ee8a4e05' ||
  budgets.baseline.cliBytes !== 1_188_036 ||
  budgets.baseline.installedJavaScriptBytes !== 2_535_107 ||
  budgets.projectResolution.p95MillisecondsMax !== 750 ||
  budgets.projectPreflight.p95MillisecondsMax !== 2_000 ||
  budgets.runtime.localColdActivationP95MillisecondsMax !== 3_000 ||
  budgets.runtime.localWarmActivationP95MillisecondsMax !== 1_000 ||
  budgets.runtime.remoteDispatchP95MillisecondsMax !== 500 ||
  budgets.runtime.streamedOutputOverheadPercentMax !== 200 ||
  budgets.runtime.idleRssIncreaseBytesMax !== 16_777_216 ||
  Object.values(budgets.lazyActivation).some(value => value !== 0)
) {
  throw new Error('Combined Developer Experience budgets drifted.');
}

for (const path of publicProjectFiles) {
  if (!packedPackageFiles.includes(path)) {
    throw new Error(`Project release file is outside the package allowlist: ${path}`);
  }
}
for (const name of [
  'startWomlRemoteLoopbackCoordinator',
  'stopWomlRemoteLoopbackCoordinator',
  'enqueueWomlRemoteDispatch',
  'prepareWomlRemoteAttempt',
  'executeWomlRemoteAttempt',
  'readWomlRemoteTerminal',
] as const) {
  if (!expectedNativeExports.includes(name)) {
    throw new Error(`CDX8 native release contract is missing ${name}.`);
  }
}

const packageContract = await read('woml-cli/package.json');
for (const marker of [
  'test:combined-developer-experience',
  'test:combined-developer-experience-package',
  'test:combined-developer-experience-release',
  'benchmark:combined-developer-experience',
]) {
  if (!packageContract.includes(marker)) {
    throw new Error(`CDX8 package contract is missing ${marker}.`);
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'Combined Developer Experience (${{ matrix.os }})',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'bun run test:combined-developer-experience-package',
  'bun scripts/benchmark-combined-developer-experience.ts',
]) {
  if (!ci.includes(marker)) throw new Error(`CDX8 CI gate is missing ${marker}.`);
}
const release = await read('.github/workflows/release.yml');
for (const marker of [
  'bun run test:combined-developer-experience',
  'verify-combined-developer-experience-release.ts',
]) {
  if (!release.includes(marker)) {
    throw new Error(`CDX8 release workflow gate is missing ${marker}.`);
  }
}

const docs = [
  await read('README.md'),
  await read('docs/getting-started.md'),
  await read('docs/cli-reference.md'),
  await read('docs/language-reference.md'),
  await read('docs/combined-developer-experience-operator-runbook.md'),
  await read('examples/README.md'),
  await read('woml-cli/README.md'),
].join('\n');
for (const marker of [
  'woml.project.json',
  'woml init',
  'woml check',
  'woml dispatch inspect',
  'woml dispatch reconcile',
  'ambiguous',
]) {
  if (!docs.includes(marker)) throw new Error(`CDX8 public docs are missing ${marker}.`);
}

const examples = await readdir(resolve(root, 'examples'), {
  recursive: true,
  withFileTypes: true,
});
if (!examples.some(entry => entry.isFile() && entry.name.endsWith('.woml'))) {
  throw new Error('CDX8 found no public WOML examples to verify.');
}

const stagedRoots = [resolve(root, 'release/main'), resolve(root, 'woml-cli/templates')];
for (const stagedRoot of stagedRoots) {
  const entries = await readdir(stagedRoot, { recursive: true, withFileTypes: true })
    .catch(() => []);
  for (const entry of entries) {
    if (!entry.isFile() || !/\.(?:js|json|md|woml)$/u.test(entry.name)) continue;
    const source = await readFile(resolve(entry.parentPath, entry.name), 'utf8');
    for (const forbidden of [
      'PRIVATE KEY-----',
      'xoxb-live-',
      'sk-live-',
      'ACTIVE_RELEASE_SECRET',
    ]) {
      if (source.includes(forbidden)) {
        throw new Error(`Release content contains forbidden credential marker ${forbidden}.`);
      }
    }
  }
}

console.log(
  `Combined Developer Experience release gate passed: ${required.length} required artifacts, ` +
  `${publicProjectFiles.length} packaged project files, ${expectedNativeExports.length} native exports, ` +
  'frozen budgets, clean-package journeys, cross-platform CI, documentation, and credential scans verified.'
);
