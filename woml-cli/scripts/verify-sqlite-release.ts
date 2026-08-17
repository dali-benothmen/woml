#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/sqliteWorkflow.woml');
const example = await Bun.file(examplePath).text();
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
if (
  compiled.schemaVersion !== 8 ||
  !compiled.graph.nodes.some(
    node => node.scriptRuntime?.bindings.includes('services') === true
  )
) {
  throw new Error(
    'SC7 verification failed: the SQLite example must compile through Model v8 and Script Bindings v1.'
  );
}

const documentation = await Bun.file(
  resolve(projectRoot, 'docs/woml-database.md')
).text();
for (const section of [
  '## Configuration and ownership',
  '## Query, execute, and CRUD',
  '## Atomic transactions',
  '## Identity, failures, and recovery',
  '## Limits and durable data',
  '## Run the example',
]) {
  if (!documentation.includes(section)) {
    throw new Error(`SC7 Database documentation is missing ${section}.`);
  }
}
const architecture = await Bun.file(
  resolve(projectRoot, 'docs/architecture.md')
).text();
const language = await Bun.file(
  resolve(projectRoot, 'docs/language-reference.md')
).text();
if (
  !architecture.includes('`services.db()` for SQLite and PostgreSQL') ||
  !language.includes('`services.db({ driver: "sqlite", connection })`')
) {
  throw new Error('SC7 architecture or language status is stale.');
}

process.stdout.write(
  '[SC7] Database v1 example, contracts, and operational documentation are publishable\n'
);

// The managed HTTP gate supplies the shared transitive checks. This SQLite
// gate adds schema, Worker, Rust, CLI, clean-package, and database hardening.
await import('./verify-http-release.ts');

process.stdout.write('[SC7] SQLite Database v1 release gate passed\n');
