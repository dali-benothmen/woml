#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> =>
  path.endsWith('.rs') ? readRustModuleSource(resolve(root, path)) : readFile(resolve(root, path), 'utf8');

const required = [
  'docs/woml-streaming-output-and-progress.md',
  'docs/protocols/streaming-output-and-progress-v1.md',
  'docs/streaming-output-and-progress-v1-threat-model.md',
  'docs/release/streaming-output-and-progress-v1-release-notes.md',
  'docs/schemas/process.v2.schema.json',
  'docs/schemas/script-host-protocol.v12.schema.json',
  'docs/schemas/service-progress.v1.schema.json',
  'docs/schemas/operational-record.v1.schema.json',
  'docs/schemas/operational-page.v1.schema.json',
  'docs/schemas/operational-stream.v1.schema.json',
  'docs/schemas/operational-output-store.v1.schema.json',
  'docs/schemas/artifact-policy.v1.schema.json',
  'examples/streamingOutput/README.md',
  'examples/streamingOutput/terraformRelease.woml',
  'examples/streamingOutput/process-policy.example.json',
  'examples/streamingOutput/artifact-policy.example.json',
  'examples/streamingOutput/performance-budgets.v1.json',
  'skills/woml/references/cli.md',
  'skills/woml/references/services.md',
  'woml-cli/tests/streaming-output-packaged-release.test.ts',
];

const sources = new Map<string, string>();
for (const path of required) {
  const source = await read(path);
  if (source.trim().length === 0) throw new Error(`${path} is empty.`);
  sources.set(path, source);
}

const budgets = JSON.parse(
  sources.get('examples/streamingOutput/performance-budgets.v1.json')!
) as Readonly<Record<string, number | string>>;
if (
  budgets.profile !== 'woml.streaming-output-performance-budgets/v1' ||
  budgets.maximumDecodedChunkBytes !== 16_384 ||
  budgets.maximumScriptFrameBytes !== 4_096 ||
  budgets.maximumReplayPageRecords !== 1_000 ||
  budgets.maximumReplayPageBytes !== 1_048_576 ||
  budgets.maximumFollowers !== 8 ||
  budgets.progressCadenceMs !== 250 ||
  Object.entries(budgets)
    .filter(([key]) => key !== 'profile')
    .some(([, value]) => typeof value !== 'number' || !Number.isFinite(value) || value <= 0)
) {
  throw new Error('Streaming Output and Progress publication budgets are invalid.');
}

const implementation = (await Promise.all([
  read('core/woml-engine/src/operational_output.rs'),
  read('core/woml-engine/src/process.rs'),
  read('core/woml-engine/src/runtime.rs'),
  read('core/woml-engine/src/artifacts.rs'),
  read('core/woml-engine/src/capability.rs'),
  read('core/woml-native/src/bridge.rs'),
  read('woml-cli/src/cli.ts'),
  read('woml-cli/src/script-host/worker.ts'),
  read('woml/src/editor.ts'),
])).join('\n');
for (const marker of [
  'MAX_OPERATIONAL_CHUNK_BYTES: usize = 16 * 1024',
  'MAX_OPERATIONAL_PAGE_RECORDS: usize = 1_000',
  'MAX_OPERATIONAL_PAGE_BYTES: usize = 1024 * 1024',
  'MAX_OPERATIONAL_FOLLOWERS: usize = 8',
  'OPERATIONAL_PROGRESS_CADENCE_MS: i64 = 250',
  'append_bounded',
  'coalesce_progress_snapshot',
  'recover_operational_output',
  'begin_operational_output',
  'AtomicBool::new(false)',
  'fetch_max',
  'ThreadsafeFunctionCallMode::NonBlocking',
  'services.progress.report',
  'streamArtifact?: WomlArtifactReferenceV1 | null',
]) {
  if (!implementation.includes(marker)) {
    throw new Error(`SOP8 authority or bounded-delivery marker is missing: ${marker}`);
  }
}

const eventVocabulary = [
  await read('core/woml-engine/src/event.rs'),
  await read('docs/schemas/run-event.v19.schema.json'),
].join('\n');
for (const forbidden of [
  'woml.operational-record/v1',
  'OperationalRecordV1',
  'OperationalBodyV1',
  'operational_output_records',
]) {
  if (eventVocabulary.includes(forbidden)) {
    throw new Error(`Operational output leaked into the durable event vocabulary: ${forbidden}`);
  }
}

for (const schemaName of [
  'process.v2.schema.json',
  'script-host-protocol.v12.schema.json',
  'service-progress.v1.schema.json',
  'operational-record.v1.schema.json',
  'operational-page.v1.schema.json',
  'operational-stream.v1.schema.json',
  'operational-output-store.v1.schema.json',
]) {
  const schema = JSON.parse(sources.get(`docs/schemas/${schemaName}`)!) as {
    readonly additionalProperties?: boolean;
    readonly oneOf?: readonly unknown[];
    readonly $defs?: Readonly<Record<string, { readonly additionalProperties?: boolean; readonly unevaluatedProperties?: boolean }>>;
  };
  const closedUnion = schema.oneOf !== undefined &&
    Object.values(schema.$defs ?? {}).every(definition =>
      definition.additionalProperties !== true && definition.unevaluatedProperties !== true
    );
  if (schema.additionalProperties !== false && !closedUnion) {
    throw new Error(`${schemaName} is not a closed public contract.`);
  }
}

const publicGuidance = [
  sources.get('docs/woml-streaming-output-and-progress.md'),
  sources.get('docs/streaming-output-and-progress-v1-threat-model.md'),
  sources.get('examples/streamingOutput/README.md'),
].join('\n').toLowerCase();
for (const marker of [
  'seven days',
  'separate from durable workflow history',
  'known resolved secret',
  'redaction cannot recognize',
  'ctrl+c',
  '--after',
  'output.sqlite',
]) {
  if (!publicGuidance.includes(marker)) {
    throw new Error(`SOP8 public operations or security guidance is missing: ${marker}`);
  }
}

const example = sources.get('examples/streamingOutput/terraformRelease.woml')!;
for (const marker of ['services.progress.report(', 'services.process.run(', 'stream: true']) {
  if (!example.includes(marker)) {
    throw new Error(`The streaming Terraform example is missing ${marker}.`);
  }
}

const nativeExports = await read('woml-cli/scripts/verify-native-exports.ts');
for (const marker of [
  'inspectWomlOperationalProgress',
  'observeWomlOperationalOutput',
  'planWomlOperationalOutputPrune',
  'readWomlOperationalOutputPage',
  'pruneWomlOperationalOutput',
]) {
  if (!nativeExports.includes(marker)) {
    throw new Error(`The native package export contract is missing ${marker}.`);
  }
}

const packageScripts = await read('woml-cli/package.json');
for (const marker of [
  'test:streaming-output-contracts',
  'test:streaming-output-hardening',
  'benchmark:streaming-output',
  'test:streaming-output-package',
  'test:streaming-output-release',
  'publication_budgets_cover_activation_append_replay_progress_concurrency_and_prune',
]) {
  if (!packageScripts.includes(marker)) {
    throw new Error(`The SOP8 release command family is missing ${marker}.`);
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'Operational Output Store (${{ matrix.os }})',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'process::tests::incremental_stream_redaction_covers_every_secret_split_and_invalid_utf8',
  'benchmark:streaming-output',
  'verify-streaming-output-release.ts',
  'streaming-output-packaged-release.test.ts',
]) {
  if (!ci.includes(marker)) throw new Error(`SOP8 CI release gate is missing: ${marker}`);
}

const release = await read('.github/workflows/release.yml');
if (!release.includes('verify-streaming-output-release.ts')) {
  throw new Error('The release workflow does not enforce the SOP8 source gate.');
}

console.log(
  `Streaming Output and Progress release gate passed: ${required.length} public artifacts, ` +
    'closed contracts, frozen budgets, body-free durable events, packaged CLI, and cross-platform CI verified.'
);
