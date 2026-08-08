#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const productExample = resolve(projectRoot, 'examples/scheduleWorkflow.woml');
const reviewedSource = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-schedule.woml'
);
const reviewedModel = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-schedule.compiled.v7.json'
);
const semanticsPath = resolve(
  projectRoot,
  'woml/tests/fixtures/schedule-semantics.v1.json'
);

const [example, source, expected, semantics] = await Promise.all([
  Bun.file(productExample).text(),
  Bun.file(reviewedSource).text(),
  Bun.file(reviewedModel).json(),
  Bun.file(semanticsPath).json(),
]);
if (example !== source) {
  throw new Error(
    'T8 verification failed: examples/scheduleWorkflow.woml must exactly match the reviewed schedule fixture.'
  );
}
const compiled = compileWoml(parseWoml(example, { file: productExample }));
if (JSON.stringify(compiled) !== JSON.stringify(expected)) {
  throw new Error(
    'T8 verification failed: the schedule product example does not deep-equal Model v7.'
  );
}
if (
  semantics.contract !== 'woml.schedule-semantics' ||
  semantics.contractVersion !== 1 ||
  semantics.dialect !== 'woml-cron-v1' ||
  !Array.isArray(semantics.occurrenceCases) ||
  semantics.occurrenceCases.length < 10
) {
  throw new Error(
    'T8 verification failed: Schedule Semantics v1 is missing required conformance cases.'
  );
}

process.stdout.write(
  '[T8] schedule example, Model v7, and Schedule Semantics v1 are pinned\n'
);

// T7 owns the prior complete release gate. Its transitive N6 verifier rebuilds
// both packages and runs every frontend, Rust, CLI, packaging, and secret test,
// including the T8 schedule compiler and deterministic semantics suites.
await import('./verify-t7.ts');

process.stdout.write('[T8] schedule compiler and time-semantics gate passed\n');
