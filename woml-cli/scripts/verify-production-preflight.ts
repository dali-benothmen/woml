#!/usr/bin/env bun

import { existsSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures');

async function filesBelow(path: string): Promise<string[]> {
  if (!existsSync(path)) return [];
  const files: string[] = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(child)));
    if (entry.isFile()) files.push(child);
  }
  return files;
}

for (const [file, required] of [
  ['docs/protocols/production-runtime-operations-v1.md', 'There is no public `woml build` command'],
  ['docs/woml-production-runtime.md', 'woml check workflows/ --config woml.runtime.json'],
  ['docs/architecture.md', '## Runtime ownership and operations'],
  ['docs/woml-production-runtime.md', 'strict production-environment preflight'],
  ['examples/production/woml.runtime.json', '"schemaVersion": 1'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`PRO1 artifact ${file} is missing ${required}.`);
  }
}

const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
const schemaFiles = (await readdir(schemaRoot))
  .filter(name => name.endsWith('.schema.json'))
  .sort();
for (const name of schemaFiles) {
  ajv.addSchema(await Bun.file(resolve(schemaRoot, name)).json());
}

const productionSchemas = [
  'runtime-configuration', 'production-preflight', 'deployment-activation',
  'background-runtime-control', 'runtime-instance', 'production-runtime-store',
  'runtime-descriptor', 'runtime-admin-http', 'runtime-operations-snapshot',
  'runtime-operations-stream', 'runtime-log-record', 'runtime-metrics',
  'runtime-health', 'backup-manifest', 'retention',
] as const;
for (const name of productionSchemas) {
  const version = name === 'production-runtime-store' ? 'v14' : 'v1';
  const id = `https://woml.dev/schemas/${name}/${version}`;
  if (ajv.getSchema(id) === undefined) throw new Error(`Missing PRO0 schema ${id}.`);
}

const requiredModels = Array.from({ length: 12 }, (_, index) => index + 1);
const requiredEvents = Array.from({ length: 11 }, (_, index) => index + 1);
const fixtureFiles = await filesBelow(fixtureRoot);
const observedModels = new Set<number>();
const observedEvents = new Set<number>();
let historicalFixtures = 0;
for (const path of fixtureFiles) {
  const modelMatch = basename(path).match(/\.compiled\.v(\d+)\.json$/);
  const eventMatch =
    path.includes('/run-events/') || path.includes('/workflow-call-events/')
      ? basename(path).match(/\.v(\d+)\.json$/)
      : basename(path).match(/(?:^|\.)(?:event|events)\.v(\d+)\.json$/);
  if (modelMatch !== null) {
    const version = Number(modelMatch[1]);
    const validate = ajv.getSchema(`https://cronflow.dev/schemas/compiled-workflow-model/v${version}`);
    const fixture = await Bun.file(path).json();
    if (validate === undefined || !validate(fixture)) {
      throw new Error(`PRO1 historical model fixture failed: ${path}: ${JSON.stringify(validate?.errors)}`);
    }
    observedModels.add(version);
    historicalFixtures += 1;
  }
  if (eventMatch !== null) {
    const version = Number(eventMatch[1]);
    const validate = ajv.getSchema(`https://cronflow.dev/schemas/run-event/v${version}`);
    const decoded = await Bun.file(path).json();
    for (const fixture of Array.isArray(decoded) ? decoded : [decoded]) {
      if (validate === undefined || !validate(fixture)) {
        throw new Error(`PRO1 historical event fixture failed: ${path}: ${JSON.stringify(validate?.errors)}`);
      }
      historicalFixtures += 1;
    }
    observedEvents.add(version);
  }
}
for (const version of requiredModels) {
  if (!observedModels.has(version)) throw new Error(`No Model v${version} fixture was verified.`);
}
for (const version of requiredEvents) {
  if (!observedEvents.has(version)) throw new Error(`No Event v${version} fixture was verified.`);
}

const contracts = await Bun.file(
  resolve(repositoryRoot, 'woml-cli/tests/fixtures/production-runtime/contracts.v1.json')
).text();
for (const [name, value] of Object.entries(process.env)) {
  if (
    name.startsWith('WOML_SECRET_') &&
    name !== 'WOML_SECRETS_PROVIDER' &&
    typeof value === 'string' &&
    value.length >= 8 &&
    contracts.includes(value)
  ) {
    throw new Error(`PRO1 reviewed fixture contains active secret ${name}.`);
  }
}

console.log(
  `[PRO1] ${schemaFiles.length} schemas compiled together; Models v1-v12, Events v1-v11, ${historicalFixtures} historical fixtures, and 15 Production Runtime contracts passed.`
);
console.log('[PRO1] Runtime configuration and non-activating production preflight gate passed.');
