#!/usr/bin/env bun

import { existsSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(import.meta.dir, '..');

for (const [file, required] of [
  [
    'WOML Durable Workflow Calls Implementation Plan.md',
    'WC7 — Harden and publish Durable Workflow Calls',
  ],
  [
    'docs/woml-workflow-calls-production.md',
    '## Security boundary',
  ],
  [
    'docs/woml-workflow-calls-production.md',
    '## Deployment checklist',
  ],
  [
    'docs/protocols/workflow-start-v1.md',
    'services.workflows.start',
  ],
  [
    'docs/schemas/workflow-start.v1.schema.json',
    'woml.workflow-start',
  ],
  [
    'docs/woml-workflow-calls.md',
    "services.workflows.call('calculate-risk'",
  ],
  [
    'core/woml-engine/tests/workflow_call_migration.rs',
    'v9_to_v10_migration_preserves_calls_definitions_and_event_histories',
  ],
  [
    'woml-cli/tests/workflow-call-packaged-release.test.ts',
    'clean consumer runs a parent and child',
  ],
  [
    'woml-cli/scripts/benchmark-workflow-calls.ts',
    'woml-workflow-calls-local-v1',
  ],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`WC7 artifact ${file} is missing ${required}.`);
  }
}

const ajv = new Ajv2020({ strict: true });
addFormats(ajv);
ajv.addKeyword({ keyword: 'x-status', schemaType: 'string', valid: true });
ajv.addKeyword({ keyword: 'x-invariants', schemaType: 'array', valid: true });
const progressSchema = await Bun.file(
  resolve(repositoryRoot, 'docs/schemas/workflow-call-progress.v1.schema.json')
).json();
ajv.compile(progressSchema);
const workflowStartSchema = await Bun.file(
  resolve(repositoryRoot, 'docs/schemas/workflow-start.v1.schema.json')
).json();
ajv.compile(workflowStartSchema);

const benchmark = Bun.spawn(
  [
    Bun.which('bun')!,
    resolve(cliRoot, 'scripts/benchmark-workflow-calls.ts'),
    '--iterations',
    '1',
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
  throw new Error(`WC7 benchmark smoke failed:\n${benchmarkStderr}`);
}
const report = JSON.parse(benchmarkStdout) as Record<string, unknown>;
for (const profile of ['sameRuntime', 'crossProcess'] as const) {
  const values = report[profile] as Record<string, unknown> | undefined;
  for (const field of [
    'sequentialTotalMs',
    'sequentialMeanMs',
    'concurrentTotalMs',
    'concurrentCallsPerSecond',
  ]) {
    const value = values?.[field];
    if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
      throw new Error(`WC7 benchmark ${profile}.${field} is invalid.`);
    }
  }
}

async function filesBelow(path: string): Promise<string[]> {
  if (!existsSync(path)) return [];
  const files: string[] = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(child)));
    else if (entry.isFile()) files.push(child);
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
        `WC7 secret scan found ${name} in packaged artifact ${basename(path)}.`
      );
    }
  }
}

process.stdout.write(
  `[WC7] benchmark smoke passed; scanned ${publicArtifacts.length} packaged artifacts` +
    (activeSecrets.length === 0
      ? ' with generated-sentinel leak coverage.\n'
      : ` against ${activeSecrets.length} active WOML secrets.\n`)
);

await import('./verify-workflow-call-composition.ts');

process.stdout.write('[WC7] Durable Workflow Calls publication gate passed\n');
