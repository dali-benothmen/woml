#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'WOML Durable Workflow Calls Implementation Plan.md',
    'WC6 — Complete composition, diagnostics, and inspection',
  ],
  [
    'docs/protocols/workflow-calls-v1.md',
    'Workflow Call Progress v1 is a separate operator surface',
  ],
  [
    'docs/woml-workflow-calls.md',
    'workflowCalls.childCalls',
  ],
  [
    'docs/schemas/workflow-call-progress.v1.schema.json',
    'woml.workflow-call-progress',
  ],
  [
    'core/woml-engine/tests/wc6_workflow_calls.rs',
    'nested_cross_process_calls_compose_with_branch_and_parallel_targets',
  ],
  [
    'woml-cli/tests/wc6_workflow-calls-cli.test.ts',
    'calls a module-backed child and inspects both durable runs',
  ],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`WC6 artifact ${file} is missing ${required}.`);
  }
}

process.stdout.write(
  '[WC6] composition, safe call progress, bounded inspection, and approval preflight are complete\n'
);

await import('./verify-wc5.ts');

process.stdout.write('[WC6] Workflow Call product-completion gate passed\n');
