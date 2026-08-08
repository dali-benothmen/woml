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

// T8 reaches the complete transitive release gate. That gate rebuilds the
// package, runs every Rust target (including fake-clock/restart/DST tests), and
// runs every CLI test in isolation (including schedule-only activation).
await import('./verify-t8.ts');

process.stdout.write('[T9] durable Rust schedule release gate passed\n');
