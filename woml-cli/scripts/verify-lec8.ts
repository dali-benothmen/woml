#!/usr/bin/env bun

import { existsSync } from 'node:fs';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(import.meta.dir, '..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');

for (const [file, required] of [
  [
    'WOML Lifecycle and Engine Controls Implementation Plan.md',
    'LEC8 — Harden and publish Lifecycle and Engine Controls',
  ],
  ['docs/woml-lifecycle-and-run-control.md', '## Cancellation semantics'],
  ['docs/woml-lifecycle-and-run-control.md', '## Production checklist'],
  ['docs/woml-notifications.md', 'Informational lifecycle notifications'],
  ['docs/woml-v0.1.md', '<lifecycle>'],
  ['docs/architecture.md', 'Event v10'],
  ['docs/woml-recovery.md', 'cancellation request'],
  ['woml-cli/README.md', '## Manage durable runs'],
  ['woml-cli/tests/lec8_packaged_release.test.ts', 'a clean consumer can execute lifecycle'],
  ['woml-cli/scripts/benchmark-lec8-lifecycle.ts', 'woml-lifecycle-controls-local-v1'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`LEC8 artifact ${file} is missing ${required}.`);
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
for (const id of [
  'https://cronflow.dev/schemas/compiled-workflow-model/v11',
  'https://cronflow.dev/schemas/run-event/v10',
  'https://woml.dev/schemas/lifecycle-binding/v1',
  'https://woml.dev/schemas/lifecycle-progress/v1',
  'https://woml.dev/schemas/run-control/v1',
  'https://woml.dev/schemas/run-inspection/v2',
  'https://woml.dev/schemas/run-list/v1',
  'https://cronflow.dev/schemas/notification-provider-host/v2',
]) {
  if (ajv.getSchema(id) === undefined) {
    throw new Error(`LEC8 schema ${id} did not compile.`);
  }
}

const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures/lifecycle');
const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();
const compiled = await Bun.file(
  resolve(fixtureRoot, 'lifecycle.compiled.v11.json')
).json();
const events = (await Bun.file(
  resolve(fixtureRoot, 'events.v10.json')
).json()) as readonly unknown[];
for (const [id, value] of [
  ['https://cronflow.dev/schemas/compiled-workflow-model/v11', compiled],
  ['https://woml.dev/schemas/lifecycle-binding/v1', contracts.binding],
  ['https://woml.dev/schemas/lifecycle-progress/v1', contracts.progress],
  ['https://woml.dev/schemas/run-control/v1', contracts.controlResult],
  ['https://woml.dev/schemas/run-list/v1', contracts.list],
  ['https://woml.dev/schemas/run-inspection/v2', contracts.inspection],
  ['https://cronflow.dev/schemas/notification-provider-host/v2', contracts.notification],
] as const) {
  const validate = ajv.getSchema(id)!;
  if (!validate(value)) {
    throw new Error(`LEC8 fixture failed ${id}: ${JSON.stringify(validate.errors)}`);
  }
}
const validateEvent = ajv.getSchema('https://cronflow.dev/schemas/run-event/v10')!;
for (const event of events) {
  if (!validateEvent(event)) {
    throw new Error(
      `LEC8 event fixture failed: ${JSON.stringify(validateEvent.errors)}`
    );
  }
}

const benchmark = Bun.spawn(
  [
    Bun.which('bun')!,
    resolve(cliRoot, 'scripts/benchmark-lec8-lifecycle.ts'),
    '--iterations',
    '5',
    '--warmup',
    '1',
  ],
  { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
);
const [benchmarkStdout, benchmarkStderr, benchmarkExit] = await Promise.all([
  new Response(benchmark.stdout).text(),
  new Response(benchmark.stderr).text(),
  benchmark.exited,
]);
if (benchmarkExit !== 0) {
  throw new Error(`LEC8 benchmark failed:\n${benchmarkStdout}${benchmarkStderr}`);
}
const report = JSON.parse(benchmarkStdout) as Record<string, unknown>;
if (
  report.benchmark !== 'woml-lifecycle-controls-local-v1' ||
  report.withinBudgets !== true
) {
  throw new Error('LEC8 benchmark did not satisfy its versioned budget report.');
}

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

const activeSecrets = Object.entries(process.env).filter(
  ([name, value]) =>
    name.startsWith('WOML_SECRET_') &&
    name !== 'WOML_SECRETS_PROVIDER' &&
    typeof value === 'string' &&
    Buffer.byteLength(value, 'utf8') >= 8
) as Array<[string, string]>;
const publicArtifacts = [
  ...(await filesBelow(resolve(cliRoot, 'dist'))),
  ...(await filesBelow(resolve(cliRoot, 'slack'))),
];
for (const [name, secret] of activeSecrets) {
  const needle = Buffer.from(secret);
  for (const path of publicArtifacts) {
    const bytes = Buffer.from(await Bun.file(path).arrayBuffer());
    if (bytes.includes(needle)) {
      throw new Error(
        `LEC8 secret scan found ${name} in packaged artifact ${basename(path)}.`
      );
    }
  }
}

const packDirectory = await mkdtemp(join(tmpdir(), 'woml-lec8-pack-audit-'));
try {
  const packed = Bun.spawnSync(
    [
      Bun.which('bun')!,
      'pm',
      'pack',
      '--ignore-scripts',
      '--destination',
      packDirectory,
    ],
    { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  if (packed.exitCode !== 0) {
    throw new Error(`LEC8 package audit failed:\n${packed.stderr.toString()}`);
  }
  const archive = (await readdir(packDirectory))
    .filter(name => name.endsWith('.tgz'))
    .map(name => resolve(packDirectory, name))[0];
  if (archive === undefined) throw new Error('LEC8 package archive is missing.');
  const listing = Bun.spawnSync(['tar', '-tzf', archive], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (listing.exitCode !== 0) {
    throw new Error(`LEC8 could not inspect its package:\n${listing.stderr.toString()}`);
  }
  const entries = listing.stdout.toString().split('\n').filter(Boolean);
  for (const required of [
    'package/package.json',
    'package/dist/cli.js',
    'package/dist/script-host.js',
    `package/dist/woml-core.${process.platform}-${process.arch}.node`,
  ]) {
    if (!entries.includes(required)) {
      throw new Error(`LEC8 package is missing ${required}.`);
    }
  }
  for (const entry of entries) {
    if (
      entry.includes('/src/') ||
      entry.includes('/tests/') ||
      entry.endsWith('.sqlite') ||
      entry.endsWith('.woml') ||
      entry.endsWith('/.env')
    ) {
      throw new Error(`LEC8 package contains private development artifact ${entry}.`);
    }
  }
} finally {
  await rm(packDirectory, { recursive: true, force: true });
}

process.stdout.write(
  `[LEC8] ${schemaFiles.length} schemas compiled; benchmark budgets passed; ` +
    `${publicArtifacts.length} packaged artifacts scanned` +
    (activeSecrets.length === 0
      ? '.\n'
      : ` against ${activeSecrets.length} active WOML secrets.\n`)
);
process.stdout.write('[LEC8] Lifecycle and Engine Controls publication gate passed\n');
