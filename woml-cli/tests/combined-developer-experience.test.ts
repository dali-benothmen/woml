import { describe, expect, test } from 'bun:test';
import { readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

import { publicProjectFiles } from '../scripts/release-contract';
import { expectedNativeExports } from '../scripts/verify-native-exports';

const root = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(
  root,
  'woml/tests/fixtures/combined-developer-experience'
);

describe('Combined Developer Experience release contract', () => {
  test('freezes the reviewed performance and package-size budgets', async () => {
    const budgets = JSON.parse(
      await readFile(resolve(fixtureRoot, 'budgets.v1.json'), 'utf8')
    );
    expect(budgets).toMatchObject({
      profile: 'woml.combined-developer-experience-budgets/v1',
      version: 1,
      baseline: {
        commit: '681ad9a80009d53bfe8853852a2443f9ee8a4e05',
        cliBytes: 1_188_036,
        installedJavaScriptBytes: 2_535_107,
      },
      runtime: {
        localColdActivationP95MillisecondsMax: 3_000,
        localWarmActivationP95MillisecondsMax: 1_000,
        remoteDispatchP95MillisecondsMax: 500,
        streamedOutputOverheadPercentMax: 200,
        idleRssIncreaseBytesMax: 16_777_216,
      },
      lazyActivation: {
        unusedProviderStartsMax: 0,
        unusedWorkerStartsMax: 0,
        unusedListenerBindsMax: 0,
        unusedNetworkRequestsMax: 0,
      },
    });
  });

  test('ships the project schemas and templates through the package allowlist', () => {
    expect(publicProjectFiles).toEqual([
      'schemas/project-check.v1.schema.json',
      'schemas/woml-project.v1.schema.json',
      'templates/workflow.json',
      'templates/AGENTS.md',
      'templates/workflows/hello.woml',
    ]);
  });

  test('keeps packaged coordinator and worker authority in the native contract', () => {
    for (const name of [
      'startWomlRemoteLoopbackCoordinator',
      'stopWomlRemoteLoopbackCoordinator',
      'enqueueWomlRemoteDispatch',
      'prepareWomlRemoteAttempt',
      'startWomlRemoteAttempt',
      'executeWomlRemoteAttempt',
      'readWomlRemoteTerminal',
    ] as const) {
      expect(expectedNativeExports).toContain(name);
    }
  });

  test('publishes every combined guide as a non-empty document', async () => {
    for (const path of [
      'docs/project-manifest.md',
      'docs/migrating-to-projects.md',
      'docs/infrastructure-agent-tutorial.md',
      'docs/combined-developer-experience-troubleshooting.md',
      'docs/combined-developer-experience-operator-runbook.md',
      'docs/remote-execution-operations.md',
    ]) {
      expect((await stat(resolve(root, path))).size, path).toBeGreaterThan(0);
    }
  });
});
