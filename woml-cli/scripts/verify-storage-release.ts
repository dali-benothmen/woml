#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/storageWorkflow.woml');
const example = await Bun.file(examplePath).text();
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
if (
  compiled.schemaVersion !== 8 ||
  compiled.graph.nodes.some(
    node => node.scriptRuntime?.bindings.includes('services') !== true
  )
) {
  throw new Error(
    'SC9 verification failed: the storage example must compile through Model v8 with Script Bindings v1.'
  );
}

const storageSchema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/storage.v1.schema.json')
).json();
const httpSchema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/managed-http.v1.schema.json')
).json();
if (
  storageSchema['x-status'] !== 'storage-contract-frozen-sc9' ||
  httpSchema['x-status'] !== 'managed-http-v1-sc9-storage-activated' ||
  !httpSchema.$defs.request.properties.responseType.enum.includes('storage')
) {
  throw new Error('SC9 Storage v1 or direct HTTP activation is not pinned.');
}

const protocol = await Bun.file(
  resolve(projectRoot, 'docs/protocols/storage-v1.md')
).text();
const httpActivation = await Bun.file(
  resolve(
    projectRoot,
    'docs/protocols/managed-http-storage-activation-v1.md'
  )
).text();
const documentation = await Bun.file(
  resolve(projectRoot, 'docs/woml-storage.md')
).text();
for (const required of [
  'Status: frozen for SC9 on 2026-08-10.',
  'Managed HTTP storage mode can',
  'Object bodies never enter operation events',
]) {
  if (!protocol.includes(required)) {
    throw new Error(`SC9 protocol artifact is missing: ${required}`);
  }
}
for (const required of [
  'Status: frozen on 2026-08-10 by SC9.',
  'does not create Managed HTTP v2',
  'body does not cross the Script Host boundary',
]) {
  if (!httpActivation.includes(required)) {
    throw new Error(`SC9 HTTP activation artifact is missing: ${required}`);
  }
}
for (const section of [
  '## Operations',
  '## Object references and durable data',
  '## Large HTTP downloads',
  '## Integrity, recovery, and failures',
  '## Run the example',
]) {
  if (!documentation.includes(section)) {
    throw new Error(`SC9 Storage documentation is missing ${section}.`);
  }
}

process.stdout.write(
  '[SC9] Storage v1 contracts, example, direct HTTP mode, and guidance are publishable\n'
);

await import('./verify-postgres-release.ts');

process.stdout.write('[SC9] durable Storage v1 release gate passed\n');
