#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/cacheWorkflow.woml');
const example = await Bun.file(examplePath).text();
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
if (
  compiled.schemaVersion !== 8 ||
  compiled.graph.nodes.some(
    node => node.scriptRuntime?.bindings.includes('services') !== true
  )
) {
  throw new Error(
    'SC10 verification failed: the cache example must compile through Model v8 with Script Bindings v1.'
  );
}

const schema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/cache.v1.schema.json')
).json();
if (
  schema['x-status'] !== 'cache-contract-frozen-sc10' ||
  !schema.$defs.request.properties.operation.enum.includes('set_if_absent')
) {
  throw new Error('SC10 Cache v1 is not pinned.');
}

const protocol = await Bun.file(
  resolve(projectRoot, 'docs/protocols/cache-v1.md')
).text();
const documentation = await Bun.file(
  resolve(projectRoot, 'docs/woml-cache.md')
).text();
for (const required of [
  'Status: frozen for SC10 on 2026-08-10.',
  'A miss is a normal successful result',
  'does not cross the frozen Capability Call v1 protocol',
  'Raw keys and values never enter operation events',
]) {
  if (!protocol.includes(required)) {
    throw new Error(`SC10 protocol artifact is missing: ${required}`);
  }
}
for (const section of [
  '## Operations and results',
  '## Workflow scope and definition updates',
  '## Expiry, eviction, and restart',
  '## Atomicity, events, and failures',
  '## Run the example',
]) {
  if (!documentation.includes(section)) {
    throw new Error(`SC10 Cache documentation is missing ${section}.`);
  }
}

const architecture = await Bun.file(
  resolve(projectRoot, 'docs/architecture.md')
).text();
const language = await Bun.file(
  resolve(projectRoot, 'docs/woml-v0.1.md')
).text();
if (
  !architecture.includes('internal named Events Service v1 are active') ||
  !language.includes('SC0–SC14 completed and hardened')
) {
  throw new Error('SC10 architecture or language status is stale.');
}

process.stdout.write(
  '[SC10] Cache v1 contracts, example, isolation, and operational guidance are publishable\n'
);

await import('./verify-storage-release.ts');

process.stdout.write('[SC10] bounded local Cache v1 release gate passed\n');
