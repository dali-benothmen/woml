#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'WOML Production Runtime and Operations Implementation Plan.md',
    'PRO3 — Durable ownership, background mode, recovery, and shutdown (completed)',
  ],
  ['core/woml-engine/src/durable.rs', 'migrate_store_v13_to_v14'],
  ['core/woml-engine/src/durable.rs', 'acquire_runtime_owner'],
  ['core/src/woml_bridge.rs', 'WOML_DEPLOYMENT_ALREADY_RUNNING'],
  ['core/woml-engine/src/webhook.rs', 'stop_with_deadline'],
  ['woml-cli/src/runtime-control.ts', 'woml.runtime-descriptor/v1'],
  ['woml-cli/src/cli.ts', 'WOML runtime started in the background.'],
  ['woml-cli/src/cli.ts', 'WOML_RUNTIME_STALE_DESCRIPTOR'],
  ['docs/woml-production-runtime.md', 'woml run workflows/ --background'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`PRO3 artifact ${file} is missing ${required}.`);
  }
}

const runtimeControl = await Bun.file(
  resolve(repositoryRoot, 'woml-cli/src/runtime-control.ts')
).text();
for (const forbidden of [
  'WOML_SECRET_',
  'SLACK_BOT_TOKEN',
  'EVENT_CONTROL_TOKEN',
]) {
  if (runtimeControl.includes(forbidden)) {
    throw new Error(
      `PRO3 runtime descriptor implementation references forbidden secret ${forbidden}.`
    );
  }
}

console.log(
  '[PRO3] Store v14 ownership, recovery-before-readiness, background handoff, exact stop, graceful drain, docs, and compatibility artifacts passed.'
);
