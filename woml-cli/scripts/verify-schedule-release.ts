#!/usr/bin/env bun

import { resolve } from 'node:path';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const semantics = await Bun.file(
  resolve(projectRoot, 'woml/tests/fixtures/schedule-semantics.v1.json')
).json();
const progressSchema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/schedule-progress.v1.schema.json')
).json();

if (
  semantics.contract !== 'woml.schedule-semantics' ||
  semantics.contractVersion !== 1 ||
  semantics.occurrenceIdentity !==
    'workflowId + triggerId + planned UTC instant' ||
  progressSchema.$id !==
    'https://cronflow.dev/schemas/schedule-progress/v1'
) {
  throw new Error(
    'T9 verification failed: schedule semantics or Schedule Progress v1 drifted.'
  );
}

process.stdout.write(
  '[T9] durable occurrence semantics and Schedule Progress v1 are pinned\n'
);

// The schedule-authoring gate rebuilds the package and supplies the fake-clock,
// restart, DST, and isolated schedule-activation checks.
await import('./verify-schedule-authoring.ts');

process.stdout.write('[T9] durable Rust schedule release gate passed\n');
