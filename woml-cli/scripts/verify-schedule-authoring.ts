#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

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
const compiled = compileWoml(parseWoml(example, { file: productExample }));
const reviewed = compileWoml(parseWoml(source, { file: reviewedSource }));
if (JSON.stringify(reviewed) !== JSON.stringify(expected)) {
  throw new Error(
    'Schedule verification failed: the reviewed source no longer deep-equals its frozen Model v7 fixture.'
  );
}
const expectedShape = {
  schemaVersion: expected.schemaVersion,
  triggers: expected.triggers,
  nodes: expected.graph.nodes.map((node: { id: string; handler: string }) => ({
    id: node.id,
    handler: node.handler,
  })),
  edges: expected.graph.edges,
};
const productShape = {
  schemaVersion: compiled.schemaVersion,
  triggers: compiled.triggers,
  nodes: compiled.graph.nodes.map(node => ({ id: node.id, handler: node.handler })),
  edges: compiled.graph.edges,
};
if (JSON.stringify(productShape) !== JSON.stringify(expectedShape)) {
  throw new Error(
    'Schedule verification failed: the public context.payload example drifted from the reviewed Model v7 schedule graph.'
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

// The Slack-trigger and notification gates supply the shared cross-layer,
// packaging, and secret checks. This gate adds schedule compilation and
// deterministic time semantics.
await import('./verify-slack-trigger-release.ts');

process.stdout.write('[T8] schedule compiler and time-semantics gate passed\n');
