#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'WOML Production Runtime and Operations Implementation Plan.md',
    'PRO5 — Observability foundation (completed)',
  ],
  ['woml-cli/src/runtime-observability.ts', 'woml.runtime-operations-snapshot/v1'],
  ['woml-cli/src/runtime-observability.ts', 'WOML_OBSERVABILITY_STREAM_GAP'],
  ['woml-cli/src/runtime-observability.ts', 'woml.runtime-log-record/v1'],
  ['woml-cli/src/runtime-observability.ts', 'woml_store_size_bytes'],
  ['core/woml-engine/src/durable.rs', 'runtime_observation_v1'],
  ['core/src/woml_bridge.rs', 'observe_woml_runtime'],
  ['woml-cli/src/runtime-control.ts', "'/readyz'"],
  ['woml-cli/src/runtime-control.ts', "'/v1/stream'"],
  ['docs/woml-observability.md', 'Failure isolation'],
  ['woml-cli/tests/fixtures/production-runtime/pro5-snapshot.v1.json', 'runtime_observability'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`PRO5 artifact ${file} is missing ${required}.`);
  }
}

const surfaces = await Promise.all(
  [
    'docs/woml-observability.md',
    'woml-cli/src/runtime-observability.ts',
    'woml-cli/tests/fixtures/production-runtime/pro5-snapshot.v1.json',
    'woml-cli/tests/fixtures/production-runtime/pro5-log.v1.json',
    'woml-cli/tests/fixtures/production-runtime/pro5-metrics.prom',
  ].map(async file => [file, await Bun.file(resolve(repositoryRoot, file)).text()] as const)
);
for (const [file, contents] of surfaces) {
  for (const forbidden of [
    'xoxb-real-secret',
    'xapp-real-secret',
    'postgres://production-secret@',
    'secret-must-not-appear',
  ]) {
    if (contents.includes(forbidden)) {
      throw new Error(`PRO5 redaction scan found forbidden text in ${file}.`);
    }
  }
}

console.log(
  '[PRO5] Snapshot, stream, logs, metrics, health, authentication, redaction, failure isolation, fixtures, and packaged runtime artifacts passed.'
);
