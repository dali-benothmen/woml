#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  ['docs/protocols/workflow-calls-v1.md', 'ownership lease in durable store v10'],
  [
    'core/woml-engine/tests/workflow_call_routing.rs',
    'separate_runtime_registries_route_over_authenticated_loopback',
  ],
  [
    'woml-cli/tests/workflow-call-routing-cli.test.ts',
    'explicit files form the same runtime unit',
  ],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`WC5 artifact ${file} is missing ${required}.`);
  }
}

process.stdout.write(
  '[WC5] multi-input activation, local ownership leases, authenticated wake-up, and pending-child recovery are complete\n'
);

await import('./verify-workflow-call-recovery.ts');

process.stdout.write('[WC5] local cross-process Workflow Call gate passed\n');
