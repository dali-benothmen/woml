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
    'WOML Runtime Policies and Durable User State Implementation Plan.md',
    'RP7 — Harden and publish Runtime Policies',
  ],
  ['docs/woml-runtime-policies.md', '## Performance and publication gate'],
  ['docs/woml-runtime-policies.md', 'WOML_POLICY_QUEUE_FULL'],
  ['docs/woml-v0.1.md', 'RP0–RP7 completed and hardened'],
  ['docs/architecture.md', 'Runtime Policies RP0–RP7 publish'],
  ['docs/woml-production-triggers.md', '## Runtime Policy backpressure'],
  ['docs/woml-workflow-calls.md', '## Runtime Policies on parent and child'],
  ['docs/woml-lifecycle-and-run-control.md', '## Runtime Policy timeout interaction'],
  ['docs/woml-recovery.md', '## Runtime Policy recovery'],
  ['docs/woml-v0.1.md', '<config concurrency='],
  ['woml-cli/README.md', '## Control workflow runtime capacity'],
  ['woml-cli/tests/rp7_packaged_release.test.ts', 'a clean consumer can check'],
  ['woml-cli/scripts/benchmark-rp7-runtime-policies.ts', 'woml-runtime-policies-local-v1'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`RP7 artifact ${file} is missing ${required}.`);
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

const schemaIds = {
  model: 'https://cronflow.dev/schemas/compiled-workflow-model/v12',
  event: 'https://cronflow.dev/schemas/run-event/v11',
  policy: 'https://woml.dev/schemas/runtime-policy/v1',
  store: 'https://woml.dev/schemas/runtime-policy-store/v12',
  claim: 'https://woml.dev/schemas/scheduler-claim/v1',
  list: 'https://woml.dev/schemas/run-list/v2',
  inspection: 'https://woml.dev/schemas/run-inspection/v3',
  progress: 'https://woml.dev/schemas/runtime-policy-progress/v1',
} as const;
for (const id of Object.values(schemaIds)) {
  if (ajv.getSchema(id) === undefined) {
    throw new Error(`RP7 schema ${id} did not compile.`);
  }
}

const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures/runtime-policies');
const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();
const compiled = await Bun.file(
  resolve(fixtureRoot, 'runtime-policy.compiled.v12.json')
).json();
const events = (await Bun.file(resolve(fixtureRoot, 'events.v11.json')).json()) as
  readonly unknown[];
for (const [id, value] of [
  [schemaIds.model, compiled],
  [schemaIds.policy, contracts.policy],
  [schemaIds.store, contracts.queuedRecord],
  [schemaIds.claim, contracts.claim],
  [schemaIds.list, contracts.list],
  [schemaIds.inspection, contracts.inspection],
  [schemaIds.progress, contracts.progress],
] as const) {
  const validate = ajv.getSchema(id)!;
  if (!validate(value)) {
    throw new Error(`RP7 fixture failed ${id}: ${JSON.stringify(validate.errors)}`);
  }
}
const validateEvent = ajv.getSchema(schemaIds.event)!;
for (const event of events) {
  if (!validateEvent(event)) {
    throw new Error(`RP7 event fixture failed: ${JSON.stringify(validateEvent.errors)}`);
  }
}

const benchmark = Bun.spawn(
  [
    Bun.which('bun')!,
    resolve(cliRoot, 'scripts/benchmark-rp7-runtime-policies.ts'),
    '--iterations',
    '2',
    '--warmup',
    '0',
  ],
  { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
);
const [benchmarkStdout, benchmarkStderr, benchmarkExit] = await Promise.all([
  new Response(benchmark.stdout).text(),
  new Response(benchmark.stderr).text(),
  benchmark.exited,
]);
if (benchmarkExit !== 0) {
  throw new Error(`RP7 benchmark failed:\n${benchmarkStdout}${benchmarkStderr}`);
}
const report = JSON.parse(benchmarkStdout) as Record<string, unknown>;
if (
  report.benchmark !== 'woml-runtime-policies-local-v1' ||
  report.withinBudgets !== true
) {
  throw new Error('RP7 benchmark did not satisfy its versioned budget report.');
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
        `RP7 secret scan found ${name} in packaged artifact ${basename(path)}.`
      );
    }
  }
}

const packDirectory = await mkdtemp(join(tmpdir(), 'woml-rp7-pack-audit-'));
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
    throw new Error(`RP7 package audit failed:\n${packed.stderr.toString()}`);
  }
  const archive = (await readdir(packDirectory))
    .filter(name => name.endsWith('.tgz'))
    .map(name => resolve(packDirectory, name))[0];
  if (archive === undefined) throw new Error('RP7 package archive is missing.');
  const listing = Bun.spawnSync(['tar', '-tzf', archive], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (listing.exitCode !== 0) {
    throw new Error(`RP7 could not inspect its package:\n${listing.stderr.toString()}`);
  }
  const entries = listing.stdout.toString().split('\n').filter(Boolean);
  for (const required of [
    'package/package.json',
    'package/dist/cli.js',
    'package/dist/script-host.js',
    `package/dist/woml-core.${process.platform}-${process.arch}.node`,
  ]) {
    if (!entries.includes(required)) {
      throw new Error(`RP7 package is missing ${required}.`);
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
      throw new Error(`RP7 package contains private development artifact ${entry}.`);
    }
  }
} finally {
  await rm(packDirectory, { recursive: true, force: true });
}

process.stdout.write(
  `[RP7] ${schemaFiles.length} schemas compiled; historical Runtime Policy ` +
    `fixtures and benchmark budgets passed; ${publicArtifacts.length} packaged artifacts scanned` +
    (activeSecrets.length === 0
      ? '.\n'
      : ` against ${activeSecrets.length} active WOML secrets.\n`)
);
process.stdout.write('[RP7] Runtime Policies publication gate passed\n');
