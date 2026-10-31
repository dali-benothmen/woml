#!/usr/bin/env bun

import { resolve } from 'node:path';

import { verifyRetryExampleContract } from './retry-example-contract';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const productExample = resolve(projectRoot, 'examples', 'retryWorkflow.woml');
const reviewedFixture = resolve(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'retry.woml'
);

const [exampleSource, fixtureSource, frozenModel] = await Promise.all([
  Bun.file(productExample).text(),
  Bun.file(reviewedFixture).text(),
  Bun.file(resolve(projectRoot, 'woml/tests/fixtures/retry.compiled.v6.json')).json(),
]);

verifyRetryExampleContract(exampleSource, fixtureSource, frozenModel);

process.stdout.write('[RI7] retry example matches the reviewed public migration; historical Model v6 is unchanged\n');

// The notification gate supplies the shared cross-layer checks. This gate adds
// the frozen retry example and packaged retry smoke in tests/cli.test.ts.
await import('./verify-slack-notification-release.ts');

process.stdout.write('[RI7] retry release gate passed\n');
