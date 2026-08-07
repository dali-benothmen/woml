#!/usr/bin/env bun

import { resolve } from 'node:path';

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

const [exampleSource, fixtureSource] = await Promise.all([
  Bun.file(productExample).text(),
  Bun.file(reviewedFixture).text(),
]);

if (exampleSource !== fixtureSource) {
  throw new Error(
    'RI7 verification failed: examples/retryWorkflow.woml must exactly match the reviewed retry fixture.'
  );
}

process.stdout.write('[RI7] reviewed retry example matches its frozen fixture\n');

// The N6 gate is the existing complete cross-layer release gate. RI7 extends
// it with the frozen retry example and the clean-package retry smoke in
// tests/cli.test.ts.
await import('./verify-n6.ts');

process.stdout.write('[RI7] retry release gate passed\n');
