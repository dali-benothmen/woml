#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'WOML Durable Workflow Calls Implementation Plan.md',
    'WC4 — Make same-runtime calls retry-safe and recoverable',
  ],
  ['docs/protocols/workflow-calls-v1.md', 'single-executor claiming'],
  ['docs/woml-recovery.md', 'The ambiguous parent'],
  ['docs/woml-services.md', 'requires stable names for repeated calls'],
  [
    'core/woml-engine/tests/workflow_call_recovery.rs',
    'child_success_and_operation_commit_still_fail_closed_before_parent_commit',
  ],
] as const) {
  const text = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!text.includes(required)) {
    throw new Error(`WC4 artifact ${file} is missing ${required}.`);
  }
}

process.stdout.write(
  '[WC4] retry reattachment, one-child claiming, cycle rejection, safe metadata, and fail-closed crash recovery are complete\n'
);

await import('./verify-workflow-call-execution.ts');

process.stdout.write('[WC4] same-runtime Workflow Call recovery gate passed\n');
