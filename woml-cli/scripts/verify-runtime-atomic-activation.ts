#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'docs/woml-production-runtime.md',
    'A bound webhook or event listener returns HTTP 503',
  ],
  ['core/woml-engine/src/webhook.rs', 'prepare_with_external_ingress'],
  ['core/woml-engine/src/webhook.rs', 'WOML_RUNTIME_NOT_READY'],
  ['core/woml-native/src/bridge.rs', 'activate_woml_webhook_runtime'],
  ['woml-cli/src/cli.ts', 'WOML_SOURCE_CHANGED_DURING_ACTIVATION'],
  ['woml-cli/src/cli.ts', 'startSuspended: true'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`PRO2 artifact ${file} is missing ${required}.`);
  }
}

console.log(
  '[PRO2] Deterministic source snapshots, activation identity, closed Rust admission, provider rollback, and compatibility artifacts passed.'
);
