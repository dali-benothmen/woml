#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

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
  resolve(projectRoot, 'docs/woml-v0.1.md')
).text();
if (
  !architecture.includes('SQLite/PostgreSQL Database v1 facade are active through SC8') ||
  !language.includes('SC0–SC8 implemented and hardened')
) {
  throw new Error('SC7 architecture or language status is stale.');
}

process.stdout.write(
  '[SC7] Database v1 example, contracts, and operational documentation are publishable\n'
);

// SC6 already reaches the complete transitive release gate. SC7 extends that
// gate through the schema, Worker, Rust, CLI, clean-package, and database
// hardening suites added by this phase.
await import('./verify-sc6.ts');

process.stdout.write('[SC7] SQLite Database v1 release gate passed\n');
