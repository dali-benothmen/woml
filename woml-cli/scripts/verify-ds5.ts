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
const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures');
const executable = resolve(cliRoot, 'dist/cli.js');

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
  [
    'WOML Runtime Policies and Durable User State Implementation Plan.md',
    'DS5 — Publish Durable User State (completed)',
  ],
  ['docs/woml-durable-state.md', 'DS5 publishes the feature'],
  ['docs/woml-data-guide.md', 'Choosing Where Workflow Data Lives'],
  ['docs/woml-data-security.md', 'WOML Local Data Security'],
  ['docs/woml-services.md', 'Data choice guide'],
  ['docs/woml-cache.md', 'Cache or durable state?'],
  ['docs/woml-storage.md', 'Storage or durable state?'],
  ['docs/woml-database.md', 'Database or durable state?'],
  ['docs/architecture.md', 'Durable User State DS0–DS5 publishes'],
  ['docs/woml-recovery.md', 'Durable User State recovery'],
  ['docs/woml-webhook-deployment.md', 'For workflows using `services.state`'],
  ['docs/woml-sdk-migration.md', 'Small durable workflow-owned memory'],
  ['woml-cli/README.md', 'Supported Durable User State'],
  ['examples/atomicCounterWorkflow.woml', 'services.state.increment'],
  ['examples/conversationStateWorkflow.woml', 'services.state.set'],
  [
    'woml-cli/tests/ds5_packaged_release.test.ts',
    'a clean consumer installs WOML',
  ],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`DS5 artifact ${file} is missing ${required}.`);
  }
}

const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
const schemaFiles = (await readdir(schemaRoot))
  .filter(name => name.endsWith('.schema.json'))
  .sort();
for (const name of schemaFiles) {
  const schema = await Bun.file(resolve(schemaRoot, name)).json();
  ajv.addSchema(schema);
}

const requiredModelVersions = Array.from({ length: 12 }, (_, index) => index + 1);
const requiredEventVersions = Array.from({ length: 11 }, (_, index) => index + 1);
for (const version of requiredModelVersions) {
  const id = `https://cronflow.dev/schemas/compiled-workflow-model/v${version}`;
  if (ajv.getSchema(id) === undefined) throw new Error(`Missing historical schema ${id}.`);
}
for (const version of requiredEventVersions) {
  const id = `https://cronflow.dev/schemas/run-event/v${version}`;
  if (ajv.getSchema(id) === undefined) throw new Error(`Missing historical schema ${id}.`);
}

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
    const validate = ajv.getSchema(
      `https://cronflow.dev/schemas/compiled-workflow-model/v${version}`
    );
    const value = await Bun.file(path).json();
    if (validate === undefined || !validate(value)) {
      throw new Error(
        `DS5 historical model fixture ${path} failed: ${JSON.stringify(validate?.errors)}`
      );
    }
    observedModels.add(version);
    historicalFixtures += 1;
  }
  if (eventMatch !== null) {
    const version = Number(eventMatch[1]);
    const validate = ajv.getSchema(
      `https://cronflow.dev/schemas/run-event/v${version}`
    );
    const decoded = await Bun.file(path).json();
    const values = Array.isArray(decoded) ? decoded : [decoded];
    for (const value of values) {
      if (validate === undefined || !validate(value)) {
        throw new Error(
          `DS5 historical event fixture ${path} failed: ${JSON.stringify(validate?.errors)}`
        );
      }
    }
    observedEvents.add(version);
    historicalFixtures += values.length;
  }
}
for (const version of requiredModelVersions) {
  if (!observedModels.has(version)) throw new Error(`No Model v${version} fixture was verified.`);
}
for (const version of requiredEventVersions) {
  if (!observedEvents.has(version)) throw new Error(`No Event v${version} fixture was verified.`);
}

const stateSchemas = {
  contract: ajv.getSchema('https://woml.dev/schemas/durable-state/v1')!,
  identity: ajv.getSchema(
    'https://woml.dev/schemas/durable-state-mutation-identity/v1'
  )!,
  metadata: ajv.getSchema('https://woml.dev/schemas/state-operation-metadata/v1')!,
  store: ajv.getSchema('https://woml.dev/schemas/durable-state-store/v13')!,
};
const stateFixtureRoot = resolve(fixtureRoot, 'durable-state');
const contracts = await Bun.file(resolve(stateFixtureRoot, 'contracts.v1.json')).json();
for (const value of [
  ...Object.values(contracts.requests),
  ...Object.values(contracts.results),
]) {
  if (!stateSchemas.contract(value)) {
    throw new Error(`DS5 State v1 fixture failed: ${JSON.stringify(stateSchemas.contract.errors)}`);
  }
}
const identity = await Bun.file(resolve(stateFixtureRoot, 'identity.v1.json')).json();
if (!stateSchemas.identity(identity)) {
  throw new Error(`DS5 state identity failed: ${JSON.stringify(stateSchemas.identity.errors)}`);
}
for (const value of await Bun.file(resolve(stateFixtureRoot, 'metadata.v1.json')).json()) {
  if (!stateSchemas.metadata(value)) {
    throw new Error(`DS5 state metadata failed: ${JSON.stringify(stateSchemas.metadata.errors)}`);
  }
}
for (const value of await Bun.file(resolve(stateFixtureRoot, 'store.v13.json')).json()) {
  if (!stateSchemas.store(value)) {
    throw new Error(`DS5 state store fixture failed: ${JSON.stringify(stateSchemas.store.errors)}`);
  }
}

const journeyDirectory = await mkdtemp(join(tmpdir(), 'woml-ds5-journey-'));
try {
  const statePath = resolve(journeyDirectory, 'state.sqlite');
  const invoke = (workflow: string) =>
    Bun.spawnSync([executable, 'test', resolve(repositoryRoot, workflow), '--state', statePath], {
      cwd: repositoryRoot,
      stdout: 'pipe',
      stderr: 'pipe',
    });
  const counterFirst = invoke('examples/atomicCounterWorkflow.woml');
  const counterSecond = invoke('examples/atomicCounterWorkflow.woml');
  const conversationFirst = invoke('examples/conversationStateWorkflow.woml');
  const conversationSecond = invoke('examples/conversationStateWorkflow.woml');
  for (const result of [counterFirst, counterSecond, conversationFirst, conversationSecond]) {
    if (result.exitCode !== 0) {
      throw new Error(`DS5 example failed:\n${result.stdout}${result.stderr}`);
    }
    if (
      result.stderr.toString().includes('processed-orders') ||
      result.stderr.toString().includes('conversation:demo-conversation')
    ) {
      throw new Error('DS5 example leaked a raw state key to diagnostics.');
    }
  }
  if (
    JSON.parse(counterFirst.stdout.toString()).processedOrders !== 1 ||
    JSON.parse(counterSecond.stdout.toString()).processedOrders !== 2 ||
    JSON.parse(conversationFirst.stdout.toString()).turns !== 1 ||
    JSON.parse(conversationSecond.stdout.toString()).turns !== 2
  ) {
    throw new Error('DS5 examples did not preserve their cross-run values.');
  }
} finally {
  await rm(journeyDirectory, { recursive: true, force: true });
}

const benchmark = Bun.spawn(
  [
    'cargo',
    'test',
    '--config',
    'profile.dev.debug=0',
    '--config',
    'profile.dev.incremental=false',
    '-j',
    '1',
    '--manifest-path',
    resolve(repositoryRoot, 'core/Cargo.toml'),
    '-p',
    'woml-engine',
    '--test',
    'ds4_durable_state_hardening',
    'state_operation_latency_and_size_stay_within_ds4_budgets',
    '--',
    '--exact',
    '--nocapture',
  ],
  { cwd: repositoryRoot, stdout: 'pipe', stderr: 'pipe' }
);
const [benchmarkStdout, benchmarkStderr, benchmarkExit] = await Promise.all([
  new Response(benchmark.stdout).text(),
  new Response(benchmark.stderr).text(),
  benchmark.exited,
]);
if (benchmarkExit !== 0 || !benchmarkStdout.includes('DS4 state benchmark:')) {
  throw new Error(`DS5 state benchmark failed:\n${benchmarkStdout}${benchmarkStderr}`);
}

const publicArtifacts = [
  ...(await filesBelow(resolve(cliRoot, 'dist'))),
  ...(await filesBelow(resolve(cliRoot, 'slack'))),
];
const activeSecrets = Object.entries(process.env).filter(
  ([name, value]) =>
    name.startsWith('WOML_SECRET_') &&
    name !== 'WOML_SECRETS_PROVIDER' &&
    typeof value === 'string' &&
    Buffer.byteLength(value, 'utf8') >= 8
) as Array<[string, string]>;
for (const [name, secret] of activeSecrets) {
  const needle = Buffer.from(secret);
  for (const path of publicArtifacts) {
    const bytes = Buffer.from(await Bun.file(path).arrayBuffer());
    if (bytes.includes(needle)) {
      throw new Error(`DS5 secret scan found ${name} in ${basename(path)}.`);
    }
  }
}

const packDirectory = await mkdtemp(join(tmpdir(), 'woml-ds5-pack-audit-'));
try {
  const packed = Bun.spawnSync(
    [Bun.which('bun')!, 'pm', 'pack', '--ignore-scripts', '--destination', packDirectory],
    { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  if (packed.exitCode !== 0) throw new Error(`DS5 package audit failed:\n${packed.stderr}`);
  const archive = (await readdir(packDirectory))
    .filter(name => name.endsWith('.tgz'))
    .map(name => resolve(packDirectory, name))[0];
  if (archive === undefined) throw new Error('DS5 package archive is missing.');
  const listing = Bun.spawnSync(['tar', '-tzf', archive], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (listing.exitCode !== 0) throw new Error(`DS5 package listing failed:\n${listing.stderr}`);
  const entries = listing.stdout.toString().split('\n').filter(Boolean);
  for (const required of [
    'package/package.json',
    'package/dist/cli.js',
    'package/dist/script-host.js',
    `package/dist/woml-core.${process.platform}-${process.arch}.node`,
  ]) {
    if (!entries.includes(required)) throw new Error(`DS5 package is missing ${required}.`);
  }
  for (const entry of entries) {
    if (
      entry.includes('/src/') ||
      entry.includes('/tests/') ||
      entry.endsWith('.sqlite') ||
      entry.endsWith('.woml') ||
      entry.endsWith('/.env')
    ) {
      throw new Error(`DS5 package contains private artifact ${entry}.`);
    }
  }
} finally {
  await rm(packDirectory, { recursive: true, force: true });
}

process.stdout.write(
  `[DS5] ${schemaFiles.length} schemas compiled together; Models v1-v12, Events v1-v11, ` +
    `${historicalFixtures} historical fixtures, State v1 fixtures, both examples, and benchmark budgets passed.\n`
);
process.stdout.write(
  `[DS5] ${publicArtifacts.length} packaged artifacts scanned` +
    (activeSecrets.length === 0
      ? '; no active WOML secrets were present.\n'
      : ` against ${activeSecrets.length} active WOML secrets.\n`)
);
process.stdout.write('[DS5] Durable User State publication gate passed\n');
