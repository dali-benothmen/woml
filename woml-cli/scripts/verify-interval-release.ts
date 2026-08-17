#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/intervalWorkflow.woml');
const progressSchemaPath = resolve(
  projectRoot,
  'docs/schemas/interval-progress.v1.schema.json'
);
const [source, progressSchema] = await Promise.all([
  Bun.file(examplePath).text(),
  Bun.file(progressSchemaPath).json(),
]);
const workflow = compileWoml(parseWoml(source, { file: examplePath }));
const interval = workflow.triggers.find(
  trigger => trigger.handler === 'trigger.interval'
);
if (
  interval?.id !== 'heartbeat' ||
  interval.config.kind !== 'object' ||
  interval.config.fields.everyMs?.kind !== 'literal' ||
  interval.config.fields.everyMs.value !== 5000 ||
  interval.config.fields.onMissed?.kind !== 'literal' ||
  interval.config.fields.onMissed.value !== 'skip' ||
  progressSchema.$id !==
    'https://cronflow.dev/schemas/interval-progress/v1'
) {
  throw new Error(
    'T10 verification failed: interval example or Interval Progress v1 drifted.'
  );
}

process.stdout.write(
  '[T10] interval example, fixed-rate lowering, and Interval Progress v1 are pinned\n'
);

// The schedule gate supplies the shared native, Rust, and isolated CLI checks;
// this gate adds fixed-rate interval execution.
await import('./verify-schedule-release.ts');

process.stdout.write('[T10] durable Rust interval release gate passed\n');
