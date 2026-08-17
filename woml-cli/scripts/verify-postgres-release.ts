#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/postgresWorkflow.woml');
const example = await Bun.file(examplePath).text();
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
const script = compiled.graph.nodes.find(node => node.id === 'recordVisit');
if (
  compiled.schemaVersion !== 8 ||
  script?.scriptRuntime?.bindings.includes('services') !== true ||
  JSON.stringify(script.scriptRuntime.requiredSecrets) !==
    JSON.stringify(['POSTGRES_URL'])
) {
  throw new Error(
    'SC8 verification failed: the PostgreSQL example must compile through Model v8 with a symbolic POSTGRES_URL dependency.'
  );
}

const schema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/database.v1.schema.json')
).json();
if (
  schema['x-status'] !==
    'database-contract-frozen-sc7-postgres-driver-activated-sc8' ||
  !schema.$defs.request.properties.driver.enum.includes('postgres')
) {
  throw new Error('SC8 Database v1 PostgreSQL activation is not pinned.');
}

const protocol = await Bun.file(
  resolve(projectRoot, 'docs/protocols/database-postgresql-activation-v1.md')
).text();
const documentation = await Bun.file(
  resolve(projectRoot, 'docs/woml-database.md')
).text();
for (const required of [
  'Status: frozen on 2026-08-10 by SC8.',
  'does not create Database v2',
  'No document/NoSQL behavior is activated',
]) {
  if (!protocol.includes(required)) {
    throw new Error(`SC8 activation artifact is missing: ${required}`);
  }
}
for (const section of [
  '## Configuration and ownership',
  '## PostgreSQL pooling and portability',
  '## Run the example',
]) {
  if (!documentation.includes(section)) {
    throw new Error(`SC8 Database documentation is missing ${section}.`);
  }
}

process.stdout.write(
  '[SC8] PostgreSQL driver contract, example, and operational guidance are publishable\n'
);

await import('./verify-sqlite-release.ts');

process.stdout.write('[SC8] SQLite/PostgreSQL Database v1 release gate passed\n');
